//! Agent composition operators.
//!
//! This module keeps the composition surface narrow: pipeline, parallel,
//! conditional selection, and mixture-of-agents aggregation all ride on the
//! existing [`crate::agent::Agent`] trait.

use async_trait::async_trait;
use futures::future::join_all;
use roko_core::{
    Body, Context, Engram, Kind, Provenance, Task, TaskCategory, TaskComplexityBand,
    TaskQualityProfile, TaskReasoningLevel, TaskSpeedPriority,
};
use std::collections::HashMap;

use crate::agent::{Agent, AgentResult};
use crate::usage::Usage;

/// How to merge parallel outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Concatenate textual outputs in input order.
    Concatenate,
    /// Aggregate outputs into a structured JSON array.
    Aggregate,
    /// Majority vote over normalized text outputs.
    Vote,
    /// Pick the single best result using a simple heuristic.
    BestOfN,
}

impl MergeStrategy {
    const fn label(self) -> &'static str {
        match self {
            Self::Concatenate => "concatenate",
            Self::Aggregate => "aggregate",
            Self::Vote => "vote",
            Self::BestOfN => "best-of-n",
        }
    }
}

/// A reusable task selector for conditional branches.
#[derive(Debug, Clone, Default)]
pub struct SkillSelector {
    default_branch: usize,
    by_category: HashMap<TaskCategory, usize>,
    by_complexity: HashMap<TaskComplexityBand, usize>,
    by_reasoning: HashMap<TaskReasoningLevel, usize>,
    by_speed: HashMap<TaskSpeedPriority, usize>,
    by_quality: HashMap<TaskQualityProfile, usize>,
}

impl SkillSelector {
    /// Create a selector with branch `0` as the fallback.
    #[must_use]
    pub fn new(default_branch: usize) -> Self {
        Self {
            default_branch,
            by_category: HashMap::new(),
            by_complexity: HashMap::new(),
            by_reasoning: HashMap::new(),
            by_speed: HashMap::new(),
            by_quality: HashMap::new(),
        }
    }

    /// Route a task category to a specific branch.
    #[must_use]
    pub fn with_category(mut self, category: TaskCategory, branch: usize) -> Self {
        self.by_category.insert(category, branch);
        self
    }

    /// Route a complexity band to a specific branch.
    #[must_use]
    pub fn with_complexity(mut self, complexity: TaskComplexityBand, branch: usize) -> Self {
        self.by_complexity.insert(complexity, branch);
        self
    }

    /// Route a reasoning level to a specific branch.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning: TaskReasoningLevel, branch: usize) -> Self {
        self.by_reasoning.insert(reasoning, branch);
        self
    }

    /// Route a latency/correctness preference to a specific branch.
    #[must_use]
    pub fn with_speed(mut self, speed: TaskSpeedPriority, branch: usize) -> Self {
        self.by_speed.insert(speed, branch);
        self
    }

    /// Route a quality profile to a specific branch.
    #[must_use]
    pub fn with_quality(mut self, quality: TaskQualityProfile, branch: usize) -> Self {
        self.by_quality.insert(quality, branch);
        self
    }

    /// Score a task into a raw branch index.
    #[must_use]
    pub fn select(&self, task: &Task) -> usize {
        if let Some(category) = task.category
            && let Some(branch) = self.by_category.get(&category)
        {
            return *branch;
        }
        if let Some(complexity) = task.complexity_band
            && let Some(branch) = self.by_complexity.get(&complexity)
        {
            return *branch;
        }
        if let Some(reasoning) = task.reasoning_level
            && let Some(branch) = self.by_reasoning.get(&reasoning)
        {
            return *branch;
        }
        if let Some(speed) = task.speed_priority
            && let Some(branch) = self.by_speed.get(&speed)
        {
            return *branch;
        }
        if let Some(quality) = task.quality_profile
            && let Some(branch) = self.by_quality.get(&quality)
        {
            return *branch;
        }

        self.default_branch
    }

    /// Convert the selector into a branch-index closure.
    #[must_use]
    pub fn into_condition(self) -> Box<dyn Fn(&Task) -> usize + Send + Sync> {
        Box::new(move |task| self.select(task))
    }
}

/// A composition of several agents.
pub enum AgentComposition {
    /// Run agents in sequence, feeding each output into the next input.
    Pipeline(Vec<Box<dyn Agent>>),
    /// Run agents concurrently and merge their results.
    Parallel(Vec<Box<dyn Agent>>, MergeStrategy),
    /// Pick one branch from the task selector.
    Conditional {
        /// Branch selector.
        condition: Box<dyn Fn(&Task) -> usize + Send + Sync>,
        /// Candidate branches.
        branches: Vec<Box<dyn Agent>>,
    },
    /// Run a candidate set, then let an aggregator compress the fan-out.
    MixtureOfAgents {
        /// Candidate agents.
        agents: Vec<Box<dyn Agent>>,
        /// Final aggregator.
        aggregator: Box<dyn Agent>,
    },
}

impl std::fmt::Debug for AgentComposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(agents) => f.debug_tuple("Pipeline").field(&agents.len()).finish(),
            Self::Parallel(agents, strategy) => f
                .debug_tuple("Parallel")
                .field(&agents.len())
                .field(strategy)
                .finish(),
            Self::Conditional { branches, .. } => f
                .debug_struct("Conditional")
                .field("branches", &branches.len())
                .finish(),
            Self::MixtureOfAgents { agents, .. } => f
                .debug_struct("MixtureOfAgents")
                .field("agents", &agents.len())
                .finish(),
        }
    }
}

impl AgentComposition {
    /// Construct a pipeline composition.
    #[must_use]
    pub fn pipeline(agents: Vec<Box<dyn Agent>>) -> Self {
        Self::Pipeline(agents)
    }

    /// Construct a parallel composition.
    #[must_use]
    pub fn parallel(agents: Vec<Box<dyn Agent>>, merge: MergeStrategy) -> Self {
        Self::Parallel(agents, merge)
    }

    /// Construct a conditional composition from a selector.
    #[must_use]
    pub fn conditional(selector: SkillSelector, branches: Vec<Box<dyn Agent>>) -> Self {
        Self::Conditional {
            condition: selector.into_condition(),
            branches,
        }
    }

    /// Construct a mixture-of-agents composition.
    #[must_use]
    pub fn mixture(agents: Vec<Box<dyn Agent>>, aggregator: Box<dyn Agent>) -> Self {
        Self::MixtureOfAgents { agents, aggregator }
    }
}

/// A named agent wrapper around an [`AgentComposition`].
pub struct CompositeAgent {
    name: String,
    composition: AgentComposition,
}

impl CompositeAgent {
    /// Create a new composite agent.
    #[must_use]
    pub fn new(name: impl Into<String>, composition: AgentComposition) -> Self {
        Self {
            name: name.into(),
            composition,
        }
    }

    fn merge_outputs(
        &self,
        input: &Engram,
        results: &[AgentResult],
        strategy: MergeStrategy,
    ) -> Engram {
        let body = match strategy {
            MergeStrategy::Concatenate => {
                let text = results
                    .iter()
                    .filter_map(|result| result.output.body.as_text().ok())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                Body::text(text)
            }
            MergeStrategy::Aggregate => {
                let items = results
                    .iter()
                    .map(|result| {
                        serde_json::json!({
                            "success": result.success,
                            "kind": result.output.kind.as_str(),
                            "content": result.output.body.as_text().ok().map(str::to_string).unwrap_or_else(|| result.output.body.kind_hint().to_string()),
                        })
                    })
                    .collect::<Vec<_>>();
                Body::Json(serde_json::Value::Array(items))
            }
            MergeStrategy::Vote => {
                let mut counts: HashMap<String, usize> = HashMap::new();
                for result in results {
                    let vote = result
                        .output
                        .body
                        .as_text()
                        .ok()
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .unwrap_or(result.output.body.kind_hint())
                        .to_ascii_lowercase();
                    *counts.entry(vote).or_insert(0) += 1;
                }
                let winner = counts
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(vote, _)| vote)
                    .unwrap_or_default();
                Body::text(winner)
            }
            MergeStrategy::BestOfN => {
                let best = results
                    .iter()
                    .enumerate()
                    .max_by_key(|(index, result)| {
                        (
                            result.success as u8,
                            result.output.body.byte_size(),
                            usize::MAX - index,
                        )
                    })
                    .map(|(_, result)| result)
                    .unwrap_or_else(|| &results[0]);
                best.output.body.clone()
            }
        };

        input
            .derive(Kind::AgentOutput, body)
            .provenance(Provenance::agent(self.name()))
            .tag("composition", strategy.label())
            .build()
    }

    fn clamp_branch(index: usize, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            index.min(len.saturating_sub(1))
        }
    }

    async fn run_pipeline(
        &self,
        agents: &[Box<dyn Agent>],
        input: &Engram,
        ctx: &Context,
    ) -> AgentResult {
        if agents.is_empty() {
            return AgentResult::fail(
                input
                    .derive(Kind::AgentOutput, Body::text("empty pipeline"))
                    .provenance(Provenance::agent(self.name()))
                    .build(),
            );
        }

        let mut current = input.clone();
        let mut trace = Vec::new();
        let mut usage = Usage::zero();
        let mut success = true;

        for agent in agents {
            let result = agent.run(&current, ctx).await;
            usage.add(&result.usage);
            trace.extend(result.trace.iter().cloned());
            trace.push(result.output.clone());
            success &= result.success;
            current = result.output.clone();
            if !result.success {
                return AgentResult {
                    output: result.output,
                    trace,
                    usage,
                    success: false,
                };
            }
        }

        AgentResult {
            output: current,
            trace,
            usage,
            success,
        }
    }

    async fn run_parallel(
        &self,
        agents: &[Box<dyn Agent>],
        input: &Engram,
        ctx: &Context,
        strategy: MergeStrategy,
    ) -> AgentResult {
        if agents.is_empty() {
            return AgentResult::fail(
                input
                    .derive(Kind::AgentOutput, Body::text("empty parallel"))
                    .provenance(Provenance::agent(self.name()))
                    .build(),
            );
        }

        let results = join_all(agents.iter().map(|agent| agent.run(input, ctx))).await;
        let mut usage = Usage::zero();
        let mut trace = Vec::new();
        let mut success = true;

        for result in &results {
            usage.add(&result.usage);
            trace.extend(result.all_signals());
            success &= result.success;
        }

        let output = self.merge_outputs(input, &results, strategy);
        AgentResult {
            output,
            trace,
            usage,
            success,
        }
    }

    async fn run_conditional(
        &self,
        condition: &(dyn Fn(&Task) -> usize + Send + Sync),
        branches: &[Box<dyn Agent>],
        input: &Engram,
        ctx: &Context,
    ) -> AgentResult {
        let task = input.body.as_json::<Task>().ok();
        let index = task.as_ref().map(condition).unwrap_or_default();
        let branch = Self::clamp_branch(index, branches.len());
        match branches.get(branch) {
            Some(agent) => agent.run(input, ctx).await,
            None => AgentResult::fail(
                input
                    .derive(Kind::AgentOutput, Body::text("missing conditional branch"))
                    .provenance(Provenance::agent(self.name()))
                    .build(),
            ),
        }
    }

    async fn run_mixture(
        &self,
        agents: &[Box<dyn Agent>],
        aggregator: &Box<dyn Agent>,
        input: &Engram,
        ctx: &Context,
    ) -> AgentResult {
        let fanout = self
            .run_parallel(agents, input, ctx, MergeStrategy::Aggregate)
            .await;
        let summary_text = fanout
            .output
            .body
            .as_text()
            .ok()
            .map(str::to_string)
            .or_else(|| serde_json::to_string(&fanout.output.body).ok())
            .unwrap_or_else(|| "[]".to_string());
        let summary_input = input
            .derive(Kind::Prompt, Body::text(summary_text))
            .provenance(Provenance::agent(self.name()))
            .tag("composition", "mixture")
            .build();

        let aggregate = aggregator.run(&summary_input, ctx).await;
        let mut usage = fanout.usage;
        usage.add(&aggregate.usage);
        let mut trace = fanout.trace;
        trace.extend(aggregate.all_signals());
        AgentResult {
            output: aggregate.output,
            trace,
            usage,
            success: fanout.success && aggregate.success,
        }
    }
}

#[async_trait]
impl Agent for CompositeAgent {
    async fn run(&self, input: &Engram, ctx: &Context) -> AgentResult {
        match &self.composition {
            AgentComposition::Pipeline(agents) => self.run_pipeline(agents, input, ctx).await,
            AgentComposition::Parallel(agents, strategy) => {
                self.run_parallel(agents, input, ctx, *strategy).await
            }
            AgentComposition::Conditional {
                condition,
                branches,
            } => {
                self.run_conditional(condition.as_ref(), branches, input, ctx)
                    .await
            }
            AgentComposition::MixtureOfAgents { agents, aggregator } => {
                self.run_mixture(agents, aggregator, input, ctx).await
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        match &self.composition {
            AgentComposition::Pipeline(agents) => {
                agents.iter().all(|agent| agent.supports_streaming())
            }
            AgentComposition::Parallel(agents, _) => {
                agents.iter().all(|agent| agent.supports_streaming())
            }
            AgentComposition::Conditional { branches, .. } => {
                branches.iter().all(|agent| agent.supports_streaming())
            }
            AgentComposition::MixtureOfAgents { agents, aggregator } => {
                agents.iter().all(|agent| agent.supports_streaming())
                    && aggregator.supports_streaming()
            }
        }
    }
}

impl std::fmt::Debug for CompositeAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeAgent")
            .field("name", &self.name)
            .field("composition", &self.composition)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAgent;
    use roko_core::{Body, Context, Engram, Kind, TaskCategory, TaskComplexityBand};

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn pipeline_feeds_output_forward() {
        let agent = CompositeAgent::new(
            "pipe",
            AgentComposition::pipeline(vec![
                Box::new(MockAgent::reply("stage-one")),
                Box::new(MockAgent::reply("stage-two")),
            ]),
        );

        let result = agent.run(&prompt("start"), &Context::at(0)).await;
        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap(), "stage-two");
    }

    #[tokio::test]
    async fn parallel_concatenates_results() {
        let agent = CompositeAgent::new(
            "parallel",
            AgentComposition::parallel(
                vec![
                    Box::new(MockAgent::reply("alpha")),
                    Box::new(MockAgent::reply("beta")),
                ],
                MergeStrategy::Concatenate,
            ),
        );

        let result = agent.run(&prompt("start"), &Context::at(0)).await;
        assert!(result.output.body.as_text().unwrap().contains("alpha"));
        assert!(result.output.body.as_text().unwrap().contains("beta"));
    }

    #[tokio::test]
    async fn conditional_uses_task_selector() {
        let selector = SkillSelector::new(0).with_category(TaskCategory::Docs, 1);
        let agent = CompositeAgent::new(
            "conditional",
            AgentComposition::conditional(
                selector,
                vec![
                    Box::new(MockAgent::reply("default")),
                    Box::new(MockAgent::reply("docs")),
                ],
            ),
        );

        let task = Task {
            category: Some(TaskCategory::Docs),
            complexity_band: Some(TaskComplexityBand::Standard),
            ..Task::new("t1", "docs")
        };
        let input = Engram::builder(Kind::Prompt)
            .body(Body::from_json(&task).unwrap())
            .build();
        let result = agent.run(&input, &Context::at(0)).await;
        assert_eq!(result.output.body.as_text().unwrap(), "docs");
    }
}
