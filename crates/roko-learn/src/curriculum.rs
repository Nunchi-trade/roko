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
}
