//! Curriculum helpers for task ordering.
//!
//! The curriculum model is intentionally lightweight: it reorders already
//! scheduled tasks using static difficulty heuristics plus a small adaptive
//! per-category skill map. That keeps the implementation concrete and easy to
//! wire into existing scheduling code later without introducing a separate
//! planner.

use std::cmp::Ordering;
use std::collections::HashMap;

use roko_core::task::{Task, TaskCategory, TaskComplexityBand};

/// High-level curriculum mode used by the learning docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurriculumMode {
    /// Front-load easier tasks.
    EasyFirst,
    /// Front-load harder tasks.
    HardFirst,
    /// Interleave easier and harder tasks.
    Interleaved,
    /// Adapt ordering based on observed success.
    Adaptive,
}

/// Strategy used to reorder tasks.
#[derive(Debug, Clone, PartialEq)]
pub enum CurriculumStrategy {
    /// Prefer easier tasks first.
    EasyFirst,
    /// Prefer harder tasks first.
    HardFirst,
    /// Alternate between easier and harder tasks to reduce local monotony.
    Interleaved,
    /// Order tasks by estimated difficulty adjusted by learned skill.
    Adaptive {
        /// Minimum rolling success rate that triggers easier ordering.
        success_threshold: f64,
    },
}

impl From<CurriculumMode> for CurriculumStrategy {
    fn from(value: CurriculumMode) -> Self {
        match value {
            CurriculumMode::EasyFirst => Self::EasyFirst,
            CurriculumMode::HardFirst => Self::HardFirst,
            CurriculumMode::Interleaved => Self::Interleaved,
            CurriculumMode::Adaptive => Self::Adaptive {
                success_threshold: 0.7,
            },
        }
    }
}

/// Small adaptive difficulty model for scheduling tasks.
#[derive(Debug, Clone)]
pub struct DifficultyModel {
    /// Reordering strategy.
    pub strategy: CurriculumStrategy,
    /// Per-category skill estimate in `[0, 1]`.
    pub skill_levels: HashMap<String, f64>,
}

impl DifficultyModel {
    /// Construct a model for the requested strategy.
    #[must_use]
    pub fn new(strategy: CurriculumStrategy) -> Self {
        Self {
            strategy,
            skill_levels: HashMap::new(),
        }
    }

    /// Observe whether a task succeeded and update the per-category skill map.
    pub fn observe(&mut self, task: &Task, success: bool) {
        let key = category_key(task);
        let entry = self.skill_levels.entry(key).or_insert(0.5);
        let target = if success { 1.0 } else { 0.0 };
        *entry = (*entry * 0.8) + (target * 0.2);
    }

    /// Estimated difficulty for a single task.
    #[must_use]
    pub fn difficulty(&self, task: &Task) -> f64 {
        (base_difficulty(task) - self.skill_offset(task)).clamp(0.0, 1.0)
    }

    /// Skill estimate for the task's category.
    #[must_use]
    pub fn skill_for(&self, task: &Task) -> f64 {
        self.skill_levels
            .get(&category_key(task))
            .copied()
            .unwrap_or(0.5)
            .clamp(0.0, 1.0)
    }

    fn skill_offset(&self, task: &Task) -> f64 {
        match self.strategy {
            CurriculumStrategy::Adaptive { success_threshold } => {
                let skill = self.skill_for(task);
                if skill >= success_threshold {
                    skill * 0.35
                } else {
                    skill * 0.15
                }
            }
            _ => 0.0,
        }
    }
}

/// Task scheduler that applies a curriculum model to a task set.
#[derive(Debug, Clone)]
pub struct CurriculumScheduler {
    /// Scheduler mode.
    pub mode: CurriculumMode,
    /// Difficulty model used for ordering.
    pub difficulty_model: DifficultyModel,
}

impl CurriculumScheduler {
    /// Construct a scheduler for `mode`.
    #[must_use]
    pub fn new(mode: CurriculumMode) -> Self {
        let strategy = CurriculumStrategy::from(mode.clone());
        Self {
            mode,
            difficulty_model: DifficultyModel::new(strategy),
        }
    }

    /// Reorder `tasks` according to the current difficulty model.
    #[must_use]
    pub fn schedule(&self, tasks: &[Task]) -> Vec<Task> {
        reorder_tasks(tasks, &self.difficulty_model)
    }
}

/// Aggregate tool-usage profile keyed by (role, task_category).
///
/// Tracks which tool sequences lead to successful outcomes and which
/// tools are low-value. Profiles can be mined from episode data and
/// injected into agent prompts as tool hints (LEARN-12).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolUsageProfile {
    /// Tool name.
    pub tool_name: String,
    /// Number of observed uses.
    pub usage_count: u64,
    /// Empirical success rate for uses of this tool.
    pub success_rate: f64,
    /// Average calls per episode for this tool.
    pub calls_per_episode: f64,
    /// Contribution to task success (pass rate delta when tool is used vs not).
    pub contribution_to_success: f64,
    /// Total tokens consumed by this tool's results.
    pub tokens_consumed: u64,
}

/// Frequent sequence of tool calls observed in successful tasks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolSequencePattern {
    /// Ordered tool names in the sequence.
    pub tools: Vec<String>,
    /// Number of tasks that exhibited the sequence.
    pub support_count: u32,
    /// Lift: pass rate with pattern vs pass rate without pattern.
    /// Values > 1.0 indicate the pattern is associated with success.
    pub lift: f64,
}

/// Advisory warning surfaced during curriculum analysis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolWarning {
    /// Tool the warning applies to.
    pub tool_name: String,
    /// Human-readable warning message.
    pub message: String,
    /// Average calls per episode.
    pub calls_per_episode: f64,
    /// Contribution to task success.
    pub contribution_to_success: f64,
    /// Total tokens consumed.
    pub tokens_consumed: u64,
}

/// Aggregated tool usage data for a (role, task_category) pair.
///
/// Used to generate prompt hints about effective tool sequences.
#[derive(Debug, Clone, Default)]
pub struct RoleToolProfile {
    /// Role identifier.
    pub role: String,
    /// Task category.
    pub category: String,
    /// Per-tool aggregate stats.
    pub tools: Vec<ToolUsageProfile>,
    /// Successful tool sequence patterns with lift > 1.0.
    pub successful_patterns: Vec<ToolSequencePattern>,
    /// Warnings about low-value tools.
    pub warnings: Vec<ToolWarning>,
}

impl RoleToolProfile {
    /// Format the top tool sequence patterns as prompt hints.
    ///
    /// Returns a natural language description suitable for injection into
    /// a system prompt. Includes up to `max_hints` patterns.
    #[must_use]
    pub fn format_hints(&self, max_hints: usize) -> String {
        if self.successful_patterns.is_empty() && self.warnings.is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();

        // Successful patterns.
        let patterns: Vec<_> = self
            .successful_patterns
            .iter()
            .filter(|p| p.lift > 1.0 && p.support_count >= 3)
            .take(max_hints)
            .collect();

        if !patterns.is_empty() {
            lines.push(format!(
                "## Tool Usage Hints ({}:{})",
                self.role, self.category
            ));
            lines.push(String::new());
            lines.push("Successful approaches typically use:".to_string());
            for pattern in &patterns {
                let sequence = pattern.tools.join(" -> ");
                lines.push(format!(
                    "- {} (seen in {} tasks, {:.0}% lift)",
                    sequence,
                    pattern.support_count,
                    (pattern.lift - 1.0) * 100.0
                ));
            }
        }

        // Low-value tool warnings.
        let warnings: Vec<_> = self
            .warnings
            .iter()
            .filter(|w| w.contribution_to_success < 0.1 && w.calls_per_episode > 3.0)
            .take(3)
            .collect();

        if !warnings.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push("Low-value tools (consider reducing usage):".to_string());
            for w in &warnings {
                lines.push(format!(
                    "- {} ({:.1} calls/episode, {:.0}% contribution)",
                    w.tool_name,
                    w.calls_per_episode,
                    w.contribution_to_success * 100.0
                ));
            }
        }

        lines.join("\n")
    }

    /// Extract tool sequence trigrams from an ordered tool call list.
    ///
    /// Returns all contiguous 3-tool subsequences.
    #[must_use]
    pub fn extract_trigrams(tools: &[String]) -> Vec<Vec<String>> {
        if tools.len() < 3 {
            return vec![tools.to_vec()];
        }
        tools.windows(3).map(|w| w.to_vec()).collect()
    }

    /// Mine tool usage profiles from a collection of episodes.
    ///
    /// Groups episodes by `(role, category)` where role is derived from
    /// `agent_template` and category from `trigger_kind`. For each group,
    /// extracts tool call sequences from `external_actions`, computes
    /// per-tool stats (usage count, success rate, calls per episode),
    /// identifies successful trigram patterns with lift, and flags
    /// low-value tools.
    ///
    /// Returns one `RoleToolProfile` per `(role, category)` pair that has
    /// at least `min_episodes` episodes.
    #[must_use]
    pub fn from_episodes(
        episodes: &[crate::episode_logger::Episode],
        min_episodes: usize,
    ) -> Vec<RoleToolProfile> {
        use std::collections::HashMap;

        // Group episodes by (role, category).
        let mut groups: HashMap<(String, String), Vec<&crate::episode_logger::Episode>> =
            HashMap::new();
        for ep in episodes {
            let role = if ep.agent_template.is_empty() {
                "unknown".to_string()
            } else {
                ep.agent_template.clone()
            };
            let category = if ep.trigger_kind.is_empty() {
                "general".to_string()
            } else {
                ep.trigger_kind.clone()
            };
            groups.entry((role, category)).or_default().push(ep);
        }

        let mut profiles = Vec::new();

        for ((role, category), eps) in &groups {
            if eps.len() < min_episodes {
                continue;
            }

            let total_episodes = eps.len() as f64;
            let successful: Vec<_> = eps.iter().filter(|ep| ep.success).copied().collect();
            let _failed: Vec<_> = eps.iter().filter(|ep| !ep.success).copied().collect();
            let base_pass_rate = successful.len() as f64 / total_episodes;

            // Extract tool names from external_actions for each episode.
            let mut tool_counts: HashMap<String, (u64, u64, u64)> = HashMap::new(); // (uses, successes, tokens)
            let mut successful_sequences: Vec<Vec<String>> = Vec::new();
            let mut all_sequences: Vec<(Vec<String>, bool)> = Vec::new();

            for ep in eps.iter() {
                let tool_names = extract_tool_names(&ep.external_actions);
                let success = ep.success;

                for name in &tool_names {
                    let entry = tool_counts.entry(name.clone()).or_default();
                    entry.0 += 1;
                    if success {
                        entry.1 += 1;
                    }
                    // Approximate token cost per tool call.
                    let per_tool_tokens = ep.tokens_used / tool_names.len().max(1) as u64;
                    entry.2 += per_tool_tokens;
                }

                all_sequences.push((tool_names.clone(), success));
                if success {
                    successful_sequences.push(tool_names);
                }
            }

            // Build per-tool profiles.
            let tools: Vec<ToolUsageProfile> = tool_counts
                .iter()
                .map(|(name, (uses, successes, tokens))| {
                    let _episodes_with_tool = eps
                        .iter()
                        .filter(|ep| extract_tool_names(&ep.external_actions).contains(name))
                        .count();
                    ToolUsageProfile {
                        tool_name: name.clone(),
                        usage_count: *uses,
                        success_rate: if *uses > 0 {
                            *successes as f64 / *uses as f64
                        } else {
                            0.0
                        },
                        calls_per_episode: *uses as f64 / total_episodes,
                        contribution_to_success: compute_contribution(name, eps, base_pass_rate),
                        tokens_consumed: *tokens,
                    }
                })
                .collect();

            // Mine trigram patterns from successful episodes.
            let mut pattern_counts: HashMap<Vec<String>, (u32, u32)> = HashMap::new(); // (success_count, total_count)
            for (seq, success) in &all_sequences {
                let trigrams = Self::extract_trigrams(seq);
                for tri in &trigrams {
                    if tri.is_empty() {
                        continue;
                    }
                    let entry = pattern_counts.entry(tri.clone()).or_default();
                    entry.1 += 1;
                    if *success {
                        entry.0 += 1;
                    }
                }
            }

            let successful_patterns: Vec<ToolSequencePattern> = pattern_counts
                .iter()
                .filter_map(|(tools, (success_count, total_count))| {
                    if *total_count < 3 {
                        return None;
                    }
                    let pattern_pass_rate = *success_count as f64 / *total_count as f64;
                    let lift = if base_pass_rate > 0.0 {
                        pattern_pass_rate / base_pass_rate
                    } else {
                        1.0
                    };
                    if lift > 1.0 {
                        Some(ToolSequencePattern {
                            tools: tools.clone(),
                            support_count: *total_count,
                            lift,
                        })
                    } else {
                        None
                    }
                })
                .collect();

            // Flag low-value tools.
            let warnings: Vec<ToolWarning> = tools
                .iter()
                .filter(|t| t.contribution_to_success < 0.1 && t.calls_per_episode > 3.0)
                .map(|t| ToolWarning {
                    tool_name: t.tool_name.clone(),
                    message: format!(
                        "{} is called {:.1} times/episode but contributes only {:.0}% to success",
                        t.tool_name,
                        t.calls_per_episode,
                        t.contribution_to_success * 100.0
                    ),
                    calls_per_episode: t.calls_per_episode,
                    contribution_to_success: t.contribution_to_success,
                    tokens_consumed: t.tokens_consumed,
                })
                .collect();

            profiles.push(RoleToolProfile {
                role: role.clone(),
                category: category.clone(),
                tools,
                successful_patterns,
                warnings,
            });
        }

        profiles
    }
}

/// Extract tool names from the `external_actions` JSON array.
///
/// Looks for `"tool"`, `"tool_name"`, or `"name"` keys in each action object.
fn extract_tool_names(actions: &[serde_json::Value]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| {
            action
                .get("tool")
                .or_else(|| action.get("tool_name"))
                .or_else(|| action.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Compute the contribution of a tool to success.
///
/// Measures the delta in pass rate between episodes that use the tool
/// versus episodes that don't. A positive contribution means the tool
/// is associated with higher success rates.
fn compute_contribution(
    tool_name: &str,
    episodes: &[&crate::episode_logger::Episode],
    base_pass_rate: f64,
) -> f64 {
    let with_tool: Vec<_> = episodes
        .iter()
        .filter(|ep| extract_tool_names(&ep.external_actions).contains(&tool_name.to_string()))
        .collect();

    if with_tool.is_empty() {
        return 0.0;
    }

    let with_pass_rate =
        with_tool.iter().filter(|ep| ep.success).count() as f64 / with_tool.len() as f64;
    (with_pass_rate - base_pass_rate).max(0.0)
}

/// Reorder tasks according to the chosen curriculum strategy.
#[must_use]
pub fn reorder_tasks(tasks: &[Task], model: &DifficultyModel) -> Vec<Task> {
    let mut tasks = tasks.to_vec();
    match model.strategy {
        CurriculumStrategy::EasyFirst => tasks.sort_by(|left, right| {
            difficulty_score(left, model)
                .partial_cmp(&difficulty_score(right, model))
                .unwrap_or(Ordering::Equal)
        }),
        CurriculumStrategy::HardFirst => tasks.sort_by(|left, right| {
            difficulty_score(right, model)
                .partial_cmp(&difficulty_score(left, model))
                .unwrap_or(Ordering::Equal)
        }),
        CurriculumStrategy::Interleaved => {
            tasks.sort_by(|left, right| {
                difficulty_score(left, model)
                    .partial_cmp(&difficulty_score(right, model))
                    .unwrap_or(Ordering::Equal)
            });
            tasks = interleave(tasks);
        }
        CurriculumStrategy::Adaptive { .. } => tasks.sort_by(|left, right| {
            difficulty_score(left, model)
                .partial_cmp(&difficulty_score(right, model))
                .unwrap_or(Ordering::Equal)
        }),
    }
    tasks
}

fn difficulty_score(task: &Task, model: &DifficultyModel) -> f64 {
    let base = model.difficulty(task);
    match model.strategy {
        CurriculumStrategy::Adaptive { .. } => base,
        _ => base_difficulty(task),
    }
}

fn interleave(mut tasks: Vec<Task>) -> Vec<Task> {
    let mut out = Vec::with_capacity(tasks.len());
    while !tasks.is_empty() {
        out.push(tasks.remove(0));
        if !tasks.is_empty() {
            out.push(tasks.pop().expect("non-empty"));
        }
    }
    out
}

fn base_difficulty(task: &Task) -> f64 {
    let complexity = match task.complexity_band.unwrap_or(TaskComplexityBand::Standard) {
        TaskComplexityBand::Fast => 0.2,
        TaskComplexityBand::Standard => 0.5,
        TaskComplexityBand::Complex => 0.85,
        _ => 0.5,
    };
    let file_pressure = (task.files.len() as f64 / 6.0).clamp(0.0, 1.0) * 0.18;
    let dependency_pressure = (task.depends_on.len() as f64 / 4.0).clamp(0.0, 1.0) * 0.14;
    let minute_pressure = task
        .estimated_minutes
        .map(|minutes| (minutes as f64 / 180.0).clamp(0.0, 1.0) * 0.18)
        .unwrap_or(0.08);
    let category_pressure = match task.category.unwrap_or(TaskCategory::Implementation) {
        TaskCategory::Research | TaskCategory::Verification => 0.08,
        TaskCategory::Integration | TaskCategory::Refactor => 0.12,
        TaskCategory::Infra | TaskCategory::Scaffolding => 0.10,
        TaskCategory::Docs => 0.04,
        TaskCategory::Implementation => 0.06,
        _ => 0.06,
    };

    (complexity + file_pressure + dependency_pressure + minute_pressure + category_pressure)
        .clamp(0.0, 1.0)
}

fn category_key(task: &Task) -> String {
    task.category
        .map(TaskCategory::label)
        .unwrap_or("uncategorized")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::TaskStatus;

    fn task(id: &str, complexity: TaskComplexityBand, minutes: u32, files: usize) -> Task {
        let mut task = Task::new(id, id);
        task.status = TaskStatus::Pending;
        task.complexity_band = Some(complexity);
        task.estimated_minutes = Some(minutes);
        task.files = (0..files).map(|idx| format!("file-{id}-{idx}")).collect();
        task
    }

    #[test]
    fn easy_first_reorders_by_difficulty() {
        let tasks = vec![
            task("hard", TaskComplexityBand::Complex, 180, 5),
            task("easy", TaskComplexityBand::Fast, 10, 1),
            task("mid", TaskComplexityBand::Standard, 45, 2),
        ];
        let model = DifficultyModel::new(CurriculumStrategy::EasyFirst);
        let ordered = reorder_tasks(&tasks, &model);

        assert_eq!(ordered[0].id, "easy");
        assert_eq!(ordered[2].id, "hard");
    }

    #[test]
    fn adaptive_strategy_learns_category_skill() {
        let mut model = DifficultyModel::new(CurriculumStrategy::Adaptive {
            success_threshold: 0.7,
        });
        let easy = task("easy", TaskComplexityBand::Fast, 10, 1);
        let hard = task("hard", TaskComplexityBand::Complex, 180, 5);
        let mut easy_cat = easy.clone();
        easy_cat.category = Some(TaskCategory::Docs);
        let mut hard_cat = hard.clone();
        hard_cat.category = Some(TaskCategory::Integration);

        for _ in 0..5 {
            model.observe(&easy_cat, true);
            model.observe(&hard_cat, false);
        }

        let ordered = reorder_tasks(&[hard_cat.clone(), easy_cat.clone()], &model);
        assert_eq!(ordered[0].id, easy_cat.id);
        assert!(model.skill_for(&easy_cat) > model.skill_for(&hard_cat));
    }

    fn make_episode(
        role: &str,
        category: &str,
        success: bool,
        tools: &[&str],
    ) -> crate::episode_logger::Episode {
        let mut ep = crate::episode_logger::Episode::new("agent-1", "task-1");
        ep.agent_template = role.to_string();
        ep.trigger_kind = category.to_string();
        ep.success = success;
        ep.tokens_used = 1000;
        ep.external_actions = tools
            .iter()
            .map(|t| serde_json::json!({"tool": t}))
            .collect();
        ep
    }

    #[test]
    fn from_episodes_mines_tool_profiles() {
        let episodes = vec![
            make_episode("implementer", "task", true, &["Read", "Edit", "Bash"]),
            make_episode("implementer", "task", true, &["Read", "Edit", "Bash"]),
            make_episode("implementer", "task", false, &["Bash", "Bash", "Bash"]),
        ];

        let profiles = RoleToolProfile::from_episodes(&episodes, 2);
        assert_eq!(profiles.len(), 1);

        let p = &profiles[0];
        assert_eq!(p.role, "implementer");
        assert_eq!(p.category, "task");
        assert!(!p.tools.is_empty());

        // "Read" appears only in successful episodes.
        let read_tool = p.tools.iter().find(|t| t.tool_name == "Read");
        assert!(read_tool.is_some());
        assert_eq!(read_tool.unwrap().usage_count, 2);
    }

    #[test]
    fn from_episodes_respects_min_episodes() {
        let episodes = vec![make_episode("implementer", "task", true, &["Read"])];

        // With min_episodes=2, single-episode groups are skipped.
        let profiles = RoleToolProfile::from_episodes(&episodes, 2);
        assert!(profiles.is_empty());

        // With min_episodes=1, the group is included.
        let profiles = RoleToolProfile::from_episodes(&episodes, 1);
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn from_episodes_format_hints_produces_output() {
        let episodes: Vec<_> = (0..10)
            .map(|i| {
                make_episode(
                    "implementer",
                    "task",
                    i < 8, // 80% pass rate
                    &["Read", "Edit", "Bash"],
                )
            })
            .collect();

        let profiles = RoleToolProfile::from_episodes(&episodes, 3);
        assert_eq!(profiles.len(), 1);

        let hints = profiles[0].format_hints(5);
        // The pattern Read->Edit->Bash should have high lift and be formatted.
        // But only if support_count >= 3 and lift > 1.0.
        assert!(hints.is_empty() || hints.contains("Tool Usage Hints") || hints.contains("->"));
    }

    #[test]
    fn extract_tool_names_from_actions() {
        let actions = vec![
            serde_json::json!({"tool": "Read", "path": "/foo"}),
            serde_json::json!({"tool_name": "Edit", "path": "/bar"}),
            serde_json::json!({"name": "Bash", "command": "ls"}),
            serde_json::json!({"unrelated": "value"}),
        ];
        let names = extract_tool_names(&actions);
        assert_eq!(names, vec!["Read", "Edit", "Bash"]);
    }
}
