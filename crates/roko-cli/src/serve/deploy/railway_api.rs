//! Railway GraphQL API deploy backend.
//!
//! Implements [`DeployBackend`] by calling Railway's GraphQL API directly
//! using `reqwest`. Mutations: `serviceCreate`, `variableCollectionUpsert`,
//! `serviceInstanceDeploy`. Queries: service status, deployment logs.

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use tracing::{debug, info};

use super::{DeployBackend, DeploySpec, Deployment, DeploymentStatus};

const RAILWAY_API_URL: &str = "https://backboard.railway.com/graphql/v2";

/// Deploy backend that calls Railway's GraphQL API directly.
pub struct RailwayApiBackend {
    client: reqwest::Client,
    api_token: String,
    project_id: String,
    environment_id: String,
}

impl RailwayApiBackend {
    /// Create a new Railway API backend with the given credentials.
    pub fn new(api_token: String, project_id: String, environment_id: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token,
            project_id,
            environment_id,
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
                        "projectId": self.project_id,
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
                        "projectId": self.project_id,
                        "environmentId": self.environment_id,
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
        let deploy_query = r"
            mutation ServiceInstanceDeploy($input: ServiceInstanceDeployV2Input!) {
                serviceInstanceDeployV2(input: $input) {
                    id
                }
            }
        ";

        let deploy_resp = self
            .graphql(
                deploy_query,
                json!({
                    "input": {
                        "serviceId": service_id,
                        "environmentId": self.environment_id,
                        "source": {
                            "image": spec.image,
                        },
                    }
                }),
            )
            .await
            .context("serviceInstanceDeployV2 failed")?;

        let deployment_id = deploy_resp["data"]["serviceInstanceDeployV2"]["id"]
            .as_str()
            .unwrap_or(&service_id)
            .to_string();

        info!(%deployment_id, "initiated Railway deployment");

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
