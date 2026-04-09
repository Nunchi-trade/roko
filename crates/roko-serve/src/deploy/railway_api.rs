//! Railway GraphQL API deploy backend.
//!
//! Implements [`DeployBackend`] by calling Railway's GraphQL API directly
//! using `reqwest`. Mutations: `serviceCreate`, `variableCollectionUpsert`,
//! `serviceInstanceDeploy`, plus project/service/volume helpers for the CLI
//! deployment flow.

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use tracing::{debug, info};

use super::{DeployBackend, DeploySpec, Deployment, DeploymentStatus};

const RAILWAY_API_URL: &str = "https://backboard.railway.com/graphql/v2";

/// High-level Railway deployment request used by the CLI wiring.
#[derive(Debug, Clone)]
pub struct RailwayDeploySpec {
    /// Human-readable Railway project name.
    pub project_name: String,
    /// Existing project ID to update, if one already exists.
    pub project_id: Option<String>,
    /// Existing environment ID to reuse, if one already exists.
    pub environment_id: Option<String>,
    /// Railway service name.
    pub service_name: String,
    /// GitHub repo slug in `owner/repo` form.
    pub repo_slug: String,
    /// Git branch to connect to.
    pub branch: String,
    /// Relative path to the Dockerfile in the repository.
    pub dockerfile_path: String,
    /// Repository root Railway should treat as the service root.
    pub root_directory: String,
    /// Healthcheck endpoint path exposed by the service.
    pub healthcheck_path: String,
    /// Persistent volume mount path.
    pub volume_mount_path: String,
    /// Optional deployment region.
    pub region: Option<String>,
    /// Environment variables to set on the service.
    pub env_vars: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct RailwayProjectContext {
    project_id: String,
    environment_id: String,
}

#[derive(Debug, Clone)]
struct RailwayProjectSnapshot {
    services: Vec<RailwayServiceSummary>,
    environments: Vec<RailwayEnvironmentSummary>,
    volumes: Vec<RailwayVolumeSummary>,
}

#[derive(Debug, Clone)]
struct RailwayServiceSummary {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct RailwayEnvironmentSummary {
    id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct RailwayVolumeSummary {
    id: String,
    name: String,
}

/// Deploy backend that calls Railway's GraphQL API directly.
pub struct RailwayApiBackend {
    client: reqwest::Client,
    api_token: String,
    project_id: Option<String>,
    environment_id: Option<String>,
}

impl RailwayApiBackend {
    /// Create a new Railway API backend with the given credentials.
    pub fn new(
        api_token: String,
        project_id: Option<String>,
        environment_id: Option<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token,
            project_id,
            environment_id,
        }
    }

    /// Query a project's services and environments.
    async fn project_snapshot(&self, project_id: &str) -> Result<RailwayProjectSnapshot> {
        let query = r"
            query ProjectSnapshot($id: String!) {
                project(id: $id) {
                    services {
                        edges {
                            node {
                                id
                                name
                            }
                        }
                    }
                    environments {
                        edges {
                            node {
                                id
                                name
                            }
                        }
                    }
                    volumes {
                        edges {
                            node {
                                id
                                name
                            }
                        }
                    }
                }
            }
        ";

        let resp = self.graphql(query, json!({ "id": project_id })).await?;
        let project = &resp["data"]["project"];

        let services = project["services"]["edges"]
            .as_array()
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|edge| {
                        let node = edge.get("node")?;
                        Some(RailwayServiceSummary {
                            id: node["id"].as_str()?.to_string(),
                            name: node["name"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let environments = project["environments"]["edges"]
            .as_array()
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|edge| {
                        let node = edge.get("node")?;
                        Some(RailwayEnvironmentSummary {
                            id: node["id"].as_str()?.to_string(),
                            name: node["name"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let volumes = project["volumes"]["edges"]
            .as_array()
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|edge| {
                        let node = edge.get("node")?;
                        Some(RailwayVolumeSummary {
                            id: node["id"].as_str()?.to_string(),
                            name: node["name"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(RailwayProjectSnapshot {
            services,
            environments,
            volumes,
        })
    }

    /// Create or update a project, then resolve the active environment.
    async fn ensure_project_context(
        &self,
        project_id: Option<&str>,
        environment_id: Option<&str>,
        project_name: &str,
    ) -> Result<RailwayProjectContext> {
        let project_id = if let Some(project_id) = project_id {
            let query = r"
                mutation ProjectUpdate($id: String!, $input: ProjectUpdateInput!) {
                    projectUpdate(id: $id, input: $input) {
                        id
                        name
                    }
                }
            ";

            self.graphql(
                query,
                json!({
                    "id": project_id,
                    "input": {
                        "name": project_name,
                    },
                }),
            )
            .await
            .context("projectUpdate failed")?;

            project_id.to_string()
        } else {
            let query = r"
                mutation ProjectCreate($input: ProjectCreateInput!) {
                    projectCreate(input: $input) {
                        id
                        name
                    }
                }
            ";

            let resp = self
                .graphql(
                    query,
                    json!({
                        "input": {
                            "name": project_name,
                        },
                    }),
                )
                .await
                .context("projectCreate failed")?;

            resp["data"]["projectCreate"]["id"]
                .as_str()
                .ok_or_else(|| anyhow!("missing project id in projectCreate response"))?
                .to_string()
        };

        let snapshot = self.project_snapshot(&project_id).await?;
        let environment_id = if let Some(environment_id) = environment_id {
            environment_id.to_string()
        } else {
            snapshot
                .environments
                .iter()
                .find(|env| env.name.eq_ignore_ascii_case("production"))
                .or_else(|| snapshot.environments.first())
                .map(|env| env.id.clone())
                .ok_or_else(|| anyhow!("project has no environments"))?
        };

        Ok(RailwayProjectContext {
            project_id,
            environment_id,
        })
    }

    /// Create or update a service connected to a GitHub repo.
    async fn ensure_service(
        &self,
        project_id: &str,
        service_name: &str,
        repo_slug: &str,
        branch: &str,
    ) -> Result<String> {
        let snapshot = self.project_snapshot(project_id).await?;
        if let Some(service) = snapshot
            .services
            .iter()
            .find(|service| service.name == service_name)
        {
            let update_query = r"
                mutation ServiceUpdate($id: String!, $input: ServiceUpdateInput!) {
                    serviceUpdate(id: $id, input: $input) {
                        id
                        name
                    }
                }
            ";

            self.graphql(
                update_query,
                json!({
                    "id": service.id,
                    "input": {
                        "name": service_name,
                    },
                }),
            )
            .await
            .context("serviceUpdate failed")?;

            let connect_query = r"
                mutation ServiceConnect($id: String!, $input: ServiceConnectInput!) {
                    serviceConnect(id: $id, input: $input) {
                        id
                    }
                }
            ";

            self.graphql(
                connect_query,
                json!({
                    "id": service.id,
                    "input": {
                        "repo": repo_slug,
                        "branch": branch,
                    },
                }),
            )
            .await
            .context("serviceConnect failed")?;

            return Ok(service.id.clone());
        }

        let create_query = r"
            mutation ServiceCreate($input: ServiceCreateInput!) {
                serviceCreate(input: $input) {
                    id
                    name
                }
            }
        ";

        let resp = self
            .graphql(
                create_query,
                json!({
                    "input": {
                        "projectId": project_id,
                        "name": service_name,
                        "source": {
                            "repo": repo_slug,
                            "branch": branch,
                        },
                    }
                }),
            )
            .await
            .context("serviceCreate failed")?;

        let service_id = resp["data"]["serviceCreate"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("missing service id in serviceCreate response"))?
            .to_string();

        info!(%service_id, name = %service_name, "created Railway service");
        Ok(service_id)
    }

    /// Configure service instance settings for the Railway deployment.
    async fn configure_service_instance(
        &self,
        service_id: &str,
        environment_id: &str,
        healthcheck_path: &str,
        dockerfile_path: &str,
        root_directory: &str,
        region: Option<&str>,
    ) -> Result<()> {
        let update_query = r"
            mutation ServiceInstanceUpdate(
                $serviceId: String!,
                $environmentId: String!,
                $input: ServiceInstanceUpdateInput!
            ) {
                serviceInstanceUpdate(
                    serviceId: $serviceId,
                    environmentId: $environmentId,
                    input: $input
                )
            }
        ";

        let mut input = json!({
            "healthcheckPath": healthcheck_path,
            "dockerfilePath": dockerfile_path,
            "rootDirectory": root_directory,
        });

        if let Some(region) = region {
            input["region"] = Value::String(region.to_string());
        }

        self.graphql(
            update_query,
            json!({
                "serviceId": service_id,
                "environmentId": environment_id,
                "input": input,
            }),
        )
        .await
        .context("serviceInstanceUpdate failed")?;

        Ok(())
    }

    /// Set service environment variables in one shot.
    async fn upsert_service_variables(
        &self,
        project_id: &str,
        environment_id: &str,
        service_id: &str,
        env_vars: &std::collections::HashMap<String, String>,
    ) -> Result<()> {
        if env_vars.is_empty() {
            return Ok(());
        }

        let vars_query = r"
            mutation VariableCollectionUpsert($input: VariableCollectionUpsertInput!) {
                variableCollectionUpsert(input: $input)
            }
        ";

        self.graphql(
            vars_query,
            json!({
                "input": {
                    "projectId": project_id,
                    "environmentId": environment_id,
                    "serviceId": service_id,
                    "variables": env_vars,
                }
            }),
        )
        .await
        .context("variableCollectionUpsert failed")?;

        Ok(())
    }

    /// Create a persistent volume mounted into the service container.
    async fn create_volume(
        &self,
        project_id: &str,
        environment_id: &str,
        service_id: &str,
        mount_path: &str,
        region: Option<&str>,
    ) -> Result<()> {
        let volume_query = r"
            mutation VolumeCreate($input: VolumeCreateInput!) {
                volumeCreate(input: $input) {
                    id
                    name
                }
            }
        ";

        let mut input = json!({
            "projectId": project_id,
            "serviceId": service_id,
            "environmentId": environment_id,
            "mountPath": mount_path,
        });
        if let Some(region) = region {
            input["region"] = Value::String(region.to_string());
        }

        self.graphql(
            volume_query,
            json!({
                "input": input,
            }),
        )
        .await
        .context("volumeCreate failed")?;

        Ok(())
    }

    /// Trigger a deployment and return the deployment ID.
    async fn trigger_deployment(
        &self,
        service_id: &str,
        environment_id: &str,
    ) -> Result<String> {
        let deploy_query = r"
            mutation ServiceInstanceDeployV2($serviceId: String!, $environmentId: String!) {
                serviceInstanceDeployV2(serviceId: $serviceId, environmentId: $environmentId)
            }
        ";

        let resp = self
            .graphql(
                deploy_query,
                json!({
                    "serviceId": service_id,
                    "environmentId": environment_id,
                }),
            )
            .await
            .context("serviceInstanceDeployV2 failed")?;

        let deployment = &resp["data"]["serviceInstanceDeployV2"];
        let deployment_id = deployment
            .as_str()
            .map(str::to_string)
            .or_else(|| deployment.get("id").and_then(|value| value.as_str().map(str::to_string)))
            .ok_or_else(|| anyhow!("missing deployment id in serviceInstanceDeployV2 response"))?;

        info!(%deployment_id, "initiated Railway deployment");
        Ok(deployment_id)
    }

    /// Deploy the Roko app, wait for healthcheck success, and return the live deployment.
    pub async fn deploy_roko_app(&self, spec: &RailwayDeploySpec) -> Result<Deployment> {
        let project = self
            .ensure_project_context(spec.project_id.as_deref(), spec.environment_id.as_deref(), &spec.project_name)
            .await?;

        let service_id = self
            .ensure_service(
                &project.project_id,
                &spec.service_name,
                &spec.repo_slug,
                &spec.branch,
            )
            .await?;

        self.configure_service_instance(
            &service_id,
            &project.environment_id,
            &spec.healthcheck_path,
            &spec.dockerfile_path,
            &spec.root_directory,
            spec.region.as_deref(),
        )
        .await?;

        self.upsert_service_variables(
            &project.project_id,
            &project.environment_id,
            &service_id,
            &spec.env_vars,
        )
        .await?;

        let snapshot = self.project_snapshot(&project.project_id).await?;
        if snapshot.volumes.is_empty() {
            self.create_volume(
                &project.project_id,
                &project.environment_id,
                &service_id,
                &spec.volume_mount_path,
                spec.region.as_deref(),
            )
            .await?;
        } else if let Some(volume) = snapshot.volumes.first() {
            info!(
                project_id = %project.project_id,
                volume_id = %volume.id,
                volume_name = %volume.name,
                "reusing existing Railway volume"
            );
        }

        let deployment_id = self
            .trigger_deployment(&service_id, &project.environment_id)
            .await?;

        self.wait_for_ready(&deployment_id, &spec.service_name).await
    }

    async fn wait_for_ready(&self, deployment_id: &str, service_name: &str) -> Result<Deployment> {
        let started_at = Utc::now();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(900);
        let mut delay = std::time::Duration::from_secs(5);

        loop {
            match self.status(deployment_id).await? {
                DeploymentStatus::Ready { url } => {
                    return Ok(Deployment {
                        id: deployment_id.to_string(),
                        name: service_name.to_string(),
                        status: DeploymentStatus::Ready { url: url.clone() },
                        url: Some(url),
                        created_at: started_at,
                    });
                }
                DeploymentStatus::Failed { reason } => {
                    anyhow::bail!("Railway deployment {deployment_id} failed: {reason}");
                }
                DeploymentStatus::TornDown => {
                    anyhow::bail!("Railway deployment {deployment_id} was torn down");
                }
                _ => {}
            }

            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for Railway deployment {deployment_id} to become ready"
                );
            }

            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(std::time::Duration::from_secs(60));
        }
    }

    /// Execute a GraphQL query/mutation against Railway API.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let resp = self
            .client
            .post(RAILWAY_API_URL)
            .bearer_auth(&self.api_token)
            .json(&json!({
                "query": query,
                "variables": variables,
            }))
            .send()
            .await
            .context("Railway API request failed")?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .context("Railway API response parse failed")?;

        if !status.is_success() {
            anyhow::bail!("Railway API error ({status}): {body}");
        }

        if let Some(errors) = body.get("errors") {
            if let Some(arr) = errors.as_array() {
                if !arr.is_empty() {
                    anyhow::bail!("Railway GraphQL errors: {errors}");
                }
            }
        }

        Ok(body)
    }
}

#[async_trait]
impl DeployBackend for RailwayApiBackend {
    async fn deploy(&self, spec: &DeploySpec) -> Result<Deployment> {
        let project_id = self
            .project_id
            .as_deref()
            .ok_or_else(|| anyhow!("railway-api backend requires project_id"))?;
        let environment_id = self
            .environment_id
            .as_deref()
            .ok_or_else(|| anyhow!("railway-api backend requires environment_id"))?;

        // Step 1: Create service
        let create_query = r"
            mutation ServiceCreate($input: ServiceCreateInput!) {
                serviceCreate(input: $input) {
                    id
                    name
                }
            }
        ";

        let create_resp = self
            .graphql(
                create_query,
                json!({
                    "input": {
                        "name": spec.name,
                        "projectId": project_id,
                        "source": {
                            "image": spec.image,
                        },
                    }
                }),
            )
            .await
            .context("serviceCreate failed")?;

        let service_id = create_resp["data"]["serviceCreate"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("missing service ID in response"))?
            .to_string();

        info!(%service_id, name = %spec.name, "created Railway service");

        // Step 2: Set environment variables
        if !spec.env_vars.is_empty() {
            let vars_query = r"
                mutation VariableCollectionUpsert($input: VariableCollectionUpsertInput!) {
                    variableCollectionUpsert(input: $input)
                }
            ";

            self.graphql(
                vars_query,
                json!({
                    "input": {
                        "projectId": project_id,
                        "environmentId": environment_id,
                        "serviceId": service_id,
                        "variables": spec.env_vars,
                    }
                }),
            )
            .await
            .context("variableCollectionUpsert failed")?;

            debug!(count = spec.env_vars.len(), "set environment variables");
        }

        // Step 3: Deploy with image
        let deployment_id = self.trigger_deployment(&service_id, environment_id).await?;

        Ok(Deployment {
            id: deployment_id,
            name: spec.name.clone(),
            status: DeploymentStatus::Creating,
            url: None,
            created_at: Utc::now(),
        })
    }

    async fn status(&self, deployment_id: &str) -> Result<DeploymentStatus> {
        let query = r"
            query DeploymentStatus($id: String!) {
                deployment(id: $id) {
                    id
                    status
                    staticUrl
                }
            }
        ";

        let resp = self.graphql(query, json!({ "id": deployment_id })).await?;
        let deployment = &resp["data"]["deployment"];
        let status_str = deployment["status"].as_str().unwrap_or("UNKNOWN");

        match status_str {
            "SUCCESS" | "READY" => {
                let url = deployment["staticUrl"]
                    .as_str()
                    .map(|u| {
                        if u.starts_with("http") {
                            u.to_string()
                        } else {
                            format!("https://{u}")
                        }
                    })
                    .unwrap_or_default();
                Ok(DeploymentStatus::Ready { url })
            }
            "BUILDING" => Ok(DeploymentStatus::Building),
            "CRASHED" | "FAILED" => Ok(DeploymentStatus::Failed {
                reason: format!("Railway deployment {status_str}"),
            }),
            "REMOVED" => Ok(DeploymentStatus::TornDown),
            // DEPLOYING, INITIALIZING, WAITING, and any unknown status
            _ => Ok(DeploymentStatus::Deploying),
        }
    }

    async fn teardown(&self, deployment_id: &str) -> Result<()> {
        // Try to delete the service (deployment_id may be a service ID)
        let query = r"
            mutation ServiceDelete($id: String!) {
                serviceDelete(id: $id)
            }
        ";

        self.graphql(query, json!({ "id": deployment_id }))
            .await
            .context("serviceDelete failed")?;

        info!(%deployment_id, "torn down Railway service");
        Ok(())
    }

    async fn logs(&self, deployment_id: &str, tail: usize) -> Result<Vec<String>> {
        let query = r"
            query DeploymentLogs($deploymentId: String!, $limit: Int) {
                deploymentLogs(deploymentId: $deploymentId, limit: $limit) {
                    message
                    timestamp
                }
            }
        ";

        let resp = self
            .graphql(
                query,
                json!({
                    "deploymentId": deployment_id,
                    "limit": i64::try_from(tail).unwrap_or(i64::MAX),
                }),
            )
            .await?;

        let logs = resp["data"]["deploymentLogs"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| entry["message"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(logs)
    }
}
