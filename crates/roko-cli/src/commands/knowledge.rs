//! knowledge command handlers.
#![allow(unused_imports)]

use crate::*;


pub(crate) async fn dispatch_knowledge(cli: &Cli, cmd: KnowledgeCmd) -> Result<i32> {
    match cmd {
        KnowledgeCmd::Query { topic, workdir } => {
            cmd_neuro(cli, NeuroCmd::Query { topic, workdir }).await
        }
        KnowledgeCmd::Stats { workdir } => cmd_neuro(cli, NeuroCmd::Stats { workdir }).await,
        KnowledgeCmd::Gc { workdir } => cmd_neuro(cli, NeuroCmd::Gc { workdir }).await,
        KnowledgeCmd::Backup {
            workdir,
            destination,
            force,
            top_n,
        } => {
            cmd_neuro(
                cli,
                NeuroCmd::Backup {
                    workdir,
                    destination,
                    force,
                    top_n,
                },
            )
            .await
        }
        KnowledgeCmd::Restore {
            workdir,
            source,
            force,
            types,
            min_confidence,
            generation,
        } => {
            cmd_neuro(
                cli,
                NeuroCmd::Restore {
                    workdir,
                    source,
                    force,
                    types,
                    min_confidence,
                    generation,
                },
            )
            .await
        }
        KnowledgeCmd::Sync {
            peer,
            workdir,
            direction,
            max_send,
        } => {
            cmd_neuro(
                cli,
                NeuroCmd::Sync {
                    peer,
                    workdir,
                    direction,
                    max_send,
                },
            )
            .await
        }
        KnowledgeCmd::Dream { cmd } => dispatch_knowledge_dream(cli, cmd).await,
        KnowledgeCmd::Custody { cmd } => {
            dispatch_knowledge_custody(cli, cmd)?;
            Ok(EXIT_SUCCESS)
        }
        KnowledgeCmd::Archive {
            older_than,
            batch_size,
            workdir,
            dry_run,
        } => cmd_archive(cli, workdir, &older_than, batch_size, dry_run).await,
    }
}


pub(crate) async fn dispatch_knowledge_dream(cli: &Cli, cmd: KnowledgeDreamCmd) -> Result<i32> {
    match cmd {
        KnowledgeDreamCmd::Run { workdir } => cmd_dream(cli, DreamCmdLegacy::Run { workdir }).await,
        KnowledgeDreamCmd::Report { workdir } => {
            cmd_dream(cli, DreamCmdLegacy::Report { workdir }).await
        }
        KnowledgeDreamCmd::Schedule { workdir } => {
            cmd_dream(cli, DreamCmdLegacy::Schedule { workdir }).await
        }
        KnowledgeDreamCmd::Journal { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let journal = roko_dreams::phase2::DreamJournal::standard(&wd);
            match journal.read_recent(limit) {
                Ok(entries) if entries.is_empty() => println!("no dream journal entries found"),
                Ok(entries) => {
                    for entry in &entries {
                        println!(
                            "[{}] cycle={} agent={} hypotheses={}/{}/{} tokens={} {}",
                            entry.cycle_start.format("%Y-%m-%d %H:%M"),
                            entry.cycle_id,
                            entry.agent_id,
                            entry.hypotheses_generated,
                            entry.hypotheses_staged,
                            entry.hypotheses_promoted,
                            entry.total_tokens,
                            if entry.early_termination {
                                "(early termination)"
                            } else {
                                ""
                            },
                        );
                    }
                    println!("\n{} entries shown (of last {})", entries.len(), limit);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!(
                        "no dream journal found at {}",
                        journal.journal_path.display()
                    );
                }
                Err(e) => return Err(e.into()),
            }
            Ok(EXIT_SUCCESS)
        }
        KnowledgeDreamCmd::Archive { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let archive = roko_dreams::phase2::DreamArchive::standard(&wd);
            match archive.read_recent(limit) {
                Ok(entries) if entries.is_empty() => println!("no dream archive entries found"),
                Ok(entries) => {
                    for entry in &entries {
                        println!(
                            "[{}] {} ({:?}) quality={:.2} -- {}",
                            entry.archived_at.format("%Y-%m-%d %H:%M"),
                            entry.entry_id,
                            entry.kind,
                            entry.quality_score,
                            entry.summary,
                        );
                    }
                    println!("\n{} entries shown (of last {})", entries.len(), limit);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!(
                        "no dream archive found at {}",
                        archive.archive_path.display()
                    );
                }
                Err(e) => return Err(e.into()),
            }
            Ok(EXIT_SUCCESS)
        }
    }
}


pub(crate) fn dispatch_knowledge_custody(cli: &Cli, cmd: KnowledgeCustodyCmd) -> Result<()> {
    match cmd {
        KnowledgeCustodyCmd::List { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_list(&wd, limit)?;
        }
        KnowledgeCustodyCmd::Show { index, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_show(&wd, index)?;
        }
        KnowledgeCustodyCmd::Verify { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_verify(&wd)?;
        }
    }
    Ok(())
}


pub(crate) async fn cmd_archive(
    cli: &Cli,
    workdir: Option<PathBuf>,
    older_than: &str,
    batch_size: usize,
    dry_run: bool,
) -> Result<i32> {
    let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let roko_dir = wd.join(".roko");
    if !roko_dir.exists() {
        bail!("no .roko/ directory found in {}", wd.display());
    }

    // Parse duration string (e.g. "30d", "7d", "24h").
    let max_age_ms = parse_duration_to_ms(older_than)
        .ok_or_else(|| anyhow!("invalid duration: {older_than} (expected e.g. '30d' or '7d')"))?;

    let cutoff_ms = chrono::Utc::now().timestamp_millis() - max_age_ms;

    // Open the hot substrate.
    let hot = roko_fs::FileSubstrate::open(&roko_dir).await?;

    // Query for old engrams.
    use roko_core::{Context, Query, Substrate};
    let ctx = Context::now();
    let query = Query::all().until(cutoff_ms).limit(batch_size);
    let candidates = hot.query(&query, &ctx).await?;

    if candidates.is_empty() {
        println!("no engrams older than {older_than} found");
        return Ok(EXIT_SUCCESS);
    }

    println!(
        "found {} engram(s) older than {older_than}{}",
        candidates.len(),
        if dry_run { " (dry run)" } else { "" }
    );

    if dry_run {
        for e in &candidates {
            let age_days = (chrono::Utc::now().timestamp_millis() - e.created_at_ms) / 86_400_000;
            println!("  {:?} | {} | {}d old", e.kind, &e.id, age_days);
        }
        return Ok(EXIT_SUCCESS);
    }

    // Confirm destructive operation (skipped in quiet / non-TTY mode).
    let prompt_msg = format!(
        "Archive {} engram(s) older than {older_than}?",
        candidates.len()
    );
    if !confirm_destructive(&prompt_msg, cli.quiet) {
        println!("aborted");
        return Ok(EXIT_SUCCESS);
    }

    // Open cold substrate and archive.
    let cold_dir = roko_dir.join("cold");
    let cold = roko_fs::ArchiveColdSubstrate::open(&cold_dir).await?;

    use roko_core::ColdSubstrate;
    let archived = cold.archive_batch(candidates.clone()).await?;

    // Prune archived engrams from hot storage.
    // Use prune with a weight threshold of f32::MAX to force-remove everything
    // below cutoff — but prune uses weight, not time. Instead we just log
    // that archival succeeded; hot-side cleanup happens via the normal prune path
    // on the next dream cycle.
    println!("archived {archived} engram(s) to {}", cold_dir.display());

    Ok(EXIT_SUCCESS)
}


/// Parse a human duration string like "30d" or "7d" or "24h" to milliseconds.
pub(crate) fn parse_duration_to_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "d" => Some(num * 24 * 3600 * 1000),
        "h" => Some(num * 3600 * 1000),
        "m" => Some(num * 60 * 1000),
        "s" => Some(num * 1000),
        _ => None,
    }
}


pub(crate) async fn cmd_neuro(cli: &Cli, cmd: NeuroCmd) -> Result<i32> {
    match cmd {
        NeuroCmd::Query { topic, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let topic = topic.join(" ");
            let topic = topic.trim().to_string();
            if topic.is_empty() {
                anyhow::bail!("provide a topic to query");
            }

            let store = KnowledgeStore::for_workdir(&wd);
            let entries = store.query(&topic, 10).with_context(|| {
                format!(
                    "query knowledge store at {} for topic '{topic}'",
                    store.path().display()
                )
            })?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "topic": topic,
                    "count": entries.len(),
                    "entries": entries,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!(
                "Knowledge matches for '{topic}' in {}:",
                store.path().display()
            );
            if entries.is_empty() {
                println!("  (no matches)");
                return Ok(EXIT_SUCCESS);
            }

            for (idx, entry) in entries.iter().enumerate() {
                println!(
                    "{}. [{}] confidence {:.2} {}",
                    idx + 1,
                    format!("{:?}", entry.kind).to_lowercase(),
                    entry.confidence.clamp(0.0, 1.0),
                    entry.content.trim()
                );
                if !entry.tags.is_empty() {
                    println!("   tags: {}", entry.tags.join(", "));
                }
                if !entry.source_episodes.is_empty() {
                    println!("   sources: {}", entry.source_episodes.join(", "));
                }
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Stats { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);
            let stats = store.stats().with_context(|| {
                format!("read knowledge store stats from {}", store.path().display())
            })?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "path": store.path(),
                    "stats": stats,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Knowledge stats for {}:", store.path().display());
            println!("  total entries: {}", stats.total_entries);
            println!("  anti-knowledge: {}", stats.anti_knowledge_count);
            println!(
                "  average confidence: {}",
                stats
                    .average_confidence
                    .map(|confidence| format!("{confidence:.3}"))
                    .unwrap_or_else(|| "n/a".to_owned())
            );
            println!("  entries by kind:");
            if stats.kind_counts.is_empty() {
                println!("    (empty)");
            } else {
                for (kind, count) in &stats.kind_counts {
                    println!("    {kind:<20} {count}");
                }
            }
            println!("  entries by tier:");
            if stats.tier_counts.is_empty() {
                println!("    (empty)");
            } else {
                for (tier, count) in &stats.tier_counts {
                    println!("    {tier:<20} {count}");
                }
            }
            if !stats.source_counts.is_empty() {
                println!("  entries by source:");
                for (source, count) in &stats.source_counts {
                    println!("    {source:<20} {count}");
                }
            }

            match stats.oldest_entry.as_ref() {
                Some(entry) => {
                    println!(
                        "  oldest entry: {} [{}] confidence {:.3} created {}",
                        entry.id,
                        format!("{:?}", entry.kind).to_lowercase(),
                        entry.confidence.clamp(0.0, 1.0),
                        entry.created_at
                    );
                }
                None => println!("  oldest entry: (none)"),
            }

            match stats.newest_entry.as_ref() {
                Some(entry) => {
                    println!(
                        "  newest entry: {} [{}] confidence {:.3} created {}",
                        entry.id,
                        format!("{:?}", entry.kind).to_lowercase(),
                        entry.confidence.clamp(0.0, 1.0),
                        entry.created_at
                    );
                }
                None => println!("  newest entry: (none)"),
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Gc { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);
            let before = store.stats().with_context(|| {
                format!("read knowledge store stats from {}", store.path().display())
            })?;
            store.gc(DEFAULT_GC_MIN_CONFIDENCE).with_context(|| {
                format!(
                    "garbage collect knowledge store at {}",
                    store.path().display()
                )
            })?;
            let after = store.stats().with_context(|| {
                format!(
                    "read knowledge store stats from {} after gc",
                    store.path().display()
                )
            })?;
            let removed = before.total_entries.saturating_sub(after.total_entries);

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "path": store.path(),
                    "threshold": DEFAULT_GC_MIN_CONFIDENCE,
                    "before": before.total_entries,
                    "after": after.total_entries,
                    "removed": removed,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Knowledge GC for {}:", store.path().display());
            println!("  threshold: {:.3}", DEFAULT_GC_MIN_CONFIDENCE);
            println!("  before: {}", before.total_entries);
            println!("  after: {}", after.total_entries);
            println!("  removed entries: {}", removed);

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Backup {
            workdir,
            destination,
            force,
            top_n,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let report = backup_neuro_store(&wd, &destination, force, top_n)?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "backup_dir": destination,
                    "knowledge_store": report.live.knowledge,
                    "knowledge_backup": report.snapshot.knowledge,
                    "confirmations_store": report.live.confirmations,
                    "confirmations_backup": report.snapshot.confirmations,
                    "confirmations_present": report.confirmations_present,
                    "top_n": top_n,
                    "entries_exported": report.entries_exported,
                    "force": force,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Neuro backup written to {}:", destination.display());
            println!("  knowledge: {}", report.snapshot.knowledge.display());
            if let Some(n) = top_n {
                println!("  genomic bottleneck: top {n} entries by confidence");
            }
            println!("  entries exported: {}", report.entries_exported);
            if report.confirmations_present {
                println!(
                    "  confirmations: {}",
                    report.snapshot.confirmations.display()
                );
            } else {
                println!("  confirmations: (none)");
            }

            // Write manifest.json alongside the backup files.
            let manifest = serde_json::json!({
                "version": 1,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "entry_count": report.entries_exported,
                "top_n": top_n,
                "source_path": report.live.knowledge,
            });
            let manifest_path = destination.join("manifest.json");
            std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
                .with_context(|| format!("write manifest to {}", manifest_path.display()))?;
            println!("  manifest: {}", manifest_path.display());

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Restore {
            workdir,
            source,
            force,
            types,
            min_confidence,
            generation,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));

            // Parse type filters if provided.
            let type_filters: Option<Vec<String>> = types.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            let report = restore_neuro_store(
                &wd,
                &source,
                force,
                generation,
                min_confidence,
                type_filters.as_deref(),
            )?;

            let confidence_decay = 0.85_f64.powi(generation as i32);

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "backup_dir": source,
                    "knowledge_store": report.live.knowledge,
                    "knowledge_backup": report.snapshot.knowledge,
                    "confirmations_store": report.live.confirmations,
                    "confirmations_backup": report.snapshot.confirmations,
                    "confirmations_present": report.confirmations_present,
                    "generation": generation,
                    "confidence_decay": confidence_decay,
                    "entries_restored": report.entries_restored,
                    "entries_filtered": report.entries_filtered,
                    "force": force,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Neuro backup restored from {}:", source.display());
            println!("  knowledge: {}", report.live.knowledge.display());
            println!("  generation: {generation} (confidence decay: {confidence_decay:.4})");
            println!("  entries restored: {}", report.entries_restored);
            println!("  entries filtered: {}", report.entries_filtered);
            println!("  tier: all restored entries set to Transient (quarantine)");
            if report.confirmations_present {
                println!("  confirmations: {}", report.live.confirmations.display());
            } else {
                println!("  confirmations: (none)");
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Sync {
            peer,
            workdir,
            direction,
            max_send,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);

            // Load the version vector from persistent state (or create empty).
            let vv_path = wd.join(".roko").join("neuro").join("version-vectors.json");
            let mut version_vectors: HashMap<String, u64> = if vv_path.exists() {
                let text = std::fs::read_to_string(&vv_path)
                    .with_context(|| format!("read version vectors from {}", vv_path.display()))?;
                serde_json::from_str(&text).unwrap_or_default()
            } else {
                HashMap::new()
            };

            let peer_seq = version_vectors.get(&peer).copied().unwrap_or(0);
            let entries = store
                .read_all()
                .with_context(|| format!("read knowledge store from {}", store.path().display()))?;

            let should_send = direction == "send" || direction == "both";
            let should_receive = direction == "receive" || direction == "both";

            let mut sent_count = 0_usize;
            let mut received_count = 0_usize;

            if should_send {
                // Build delta: entries newer than peer's last-seen sequence.
                // Use entry index as a proxy sequence number for local ordering.
                let delta: Vec<_> = entries
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| (*idx as u64) > peer_seq)
                    .take(max_send)
                    .collect();
                sent_count = delta.len();

                // Write delta to an outbox file for the peer.
                if !delta.is_empty() {
                    let outbox_dir = wd.join(".roko").join("mesh").join("outbox");
                    std::fs::create_dir_all(&outbox_dir)?;
                    let delta_path = outbox_dir.join(format!("delta-{peer}.jsonl"));
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&delta_path)?;
                    for (_, entry) in &delta {
                        let line = serde_json::to_string(entry)?;
                        use std::io::Write;
                        writeln!(f, "{line}")?;
                    }
                    println!("  outbox: {}", delta_path.display());
                }
            }

            if should_receive {
                // Check inbox for incoming deltas from the peer.
                let inbox_dir = wd.join(".roko").join("mesh").join("inbox");
                let inbox_path = inbox_dir.join(format!("delta-{peer}.jsonl"));
                if inbox_path.exists() {
                    let text = std::fs::read_to_string(&inbox_path)?;
                    let mut imported = Vec::new();
                    for line in text.lines() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if let Ok(mut entry) =
                            serde_json::from_str::<roko_neuro::KnowledgeEntry>(line)
                        {
                            // Apply received confidence discount (0.7x).
                            entry.confidence *= 0.7;
                            entry.tier = roko_neuro::KnowledgeTier::Transient;
                            entry.source = Some(format!("mesh:{peer}"));
                            imported.push(entry);
                        }
                    }
                    received_count = imported.len();
                    if !imported.is_empty() {
                        store.ingest(imported).with_context(|| {
                            format!("import mesh entries from {}", inbox_path.display())
                        })?;
                    }
                    // Clean up processed inbox file.
                    let _ = std::fs::remove_file(&inbox_path);
                }
            }

            // Update version vector for this peer.
            let new_seq = entries.len() as u64;
            version_vectors.insert(peer.clone(), new_seq);
            if let Some(parent) = vv_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&vv_path, serde_json::to_string_pretty(&version_vectors)?)?;

            if cli.json {
                let payload = serde_json::json!({
                    "peer": peer,
                    "direction": direction,
                    "sent": sent_count,
                    "received": received_count,
                    "local_seq": new_seq,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Mesh sync with peer '{peer}':");
            println!("  direction: {direction}");
            println!("  sent: {sent_count} engrams");
            println!("  received: {received_count} engrams (0.7x confidence discount)");
            println!("  local sequence: {new_seq}");

            Ok(EXIT_SUCCESS)
        }
    }
}


pub(crate) async fn cmd_dream(cli: &Cli, cmd: DreamCmdLegacy) -> Result<i32> {
    match cmd {
        DreamCmdLegacy::Run { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            prepare_runtime_hooks(&workdir, cli.quiet);

            let mut runner = build_dream_runner(cli, &workdir)?;
            let report = match runner.consolidate() {
                Ok(report) => report,
                Err(e) => {
                    // Appraise dream failure into the daimon affect state.
                    use roko_daimon::{AffectEngine as _, AffectEvent, DaimonState};
                    let daimon_path = workdir.join(".roko").join("daimon").join("affect.json");
                    let mut daimon = DaimonState::load_or_new(&daimon_path);
                    let _ = daimon.appraise(AffectEvent::DreamFailure {
                        task_type: "consolidation".to_string(),
                        failure_count: 1,
                    });
                    return Err(e);
                }
            };
            let cfactor_snapshot = refresh_cfactor_snapshot(workdir.join(".roko").join("learn"))
                .await
                .map_err(|e| anyhow!("refresh c-factor snapshot: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if !cli.quiet {
                println!(
                    "dream cycle completed: {} episodes, {} clusters, {} knowledge entries, {} playbooks",
                    report.processed_episodes,
                    report.clusters.len(),
                    report.knowledge_entries_written,
                    report.playbooks_created
                );
                if let Some(processed_through) = report.processed_through {
                    println!("processed through: {processed_through}");
                }
                println!(
                    "report saved under: {}",
                    workdir.join(".roko").join("dreams").display()
                );
                println!("c-factor: {:.3}", cfactor_snapshot.overall);
            }

            Ok(EXIT_SUCCESS)
        }
        DreamCmdLegacy::Report { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &workdir)?;
            let report = runner.latest_report()?.ok_or_else(|| {
                anyhow!(
                    "no dream report found in {}",
                    workdir.join(".roko").join("dreams").display()
                )
            })?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "dream report: {} episodes, {} clusters, {} knowledge entries, {} playbooks",
                    report.processed_episodes,
                    report.clusters.len(),
                    report.knowledge_entries_written,
                    report.playbooks_created
                );
                println!("started: {}", report.started_at);
                println!("completed: {}", report.completed_at);
                if let Some(processed_through) = report.processed_through {
                    println!("processed through: {processed_through}");
                }
            }
            Ok(EXIT_SUCCESS)
        }
        DreamCmdLegacy::Schedule { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &workdir)?;
            let schedule = runner.schedule_next();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "next_fire_seconds": schedule.map(|duration| duration.as_secs())
                    })
                );
            } else if let Some(duration) = schedule {
                println!("next dream in {:?}", duration);
            } else {
                println!("no dream scheduled");
            }
            Ok(EXIT_SUCCESS)
        }
    }
}


pub(crate) fn build_dream_runner(cli: &Cli, workdir: &Path) -> Result<DreamRunner> {
    let cli_config = resolve_config_for_workdir(cli, workdir)?;
    Ok(DreamRunner::new(
        workdir.to_path_buf(),
        DreamLoopConfig {
            auto_dream: cli_config.dreams.auto_dream,
            idle_threshold_mins: cli_config.dreams.idle_threshold_mins,
            min_episodes_for_dream: cli_config.dreams.min_episodes_for_dream,
            agent: DreamAgentConfig {
                command: cli_config.agent.command.clone(),
                args: cli_config.agent.args.clone(),
                model: cli_config.agent.model.clone(),
                bare_mode: cli_config.agent.bare_mode,
                effort: cli_config.agent.effort.clone(),
                fallback_model: cli_config.agent.fallback_model.clone(),
                timeout_ms: cli_config.agent.timeout_ms,
                env: cli_config.agent.env.clone(),
            },
        },
    ))
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeuroTransferReport {
    pub(crate) live: NeuroFileSet,
    pub(crate) snapshot: NeuroFileSet,
    pub(crate) confirmations_present: bool,
    /// Number of entries exported (only relevant for backup with --top-n).
    pub(crate) entries_exported: usize,
    /// Number of entries restored (only relevant for restore).
    pub(crate) entries_restored: usize,
    /// Number of entries filtered out during restore.
    pub(crate) entries_filtered: usize,
}

pub(crate) fn backup_neuro_store(
    workdir: &Path,
    destination: &Path,
    force: bool,
    top_n: Option<usize>,
) -> Result<NeuroTransferReport> {
    let live = neuro_live_files(workdir);
    let snapshot = neuro_snapshot_files(destination);

    if let Some(n) = top_n {
        // Genomic bottleneck: export only the top N entries by confidence.
        let store = KnowledgeStore::for_workdir(workdir);
        let mut entries = store
            .read_all()
            .with_context(|| format!("read knowledge store from {}", store.path().display()))?;
        // Sort by confidence descending.
        entries.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(n);

        // Write entries to the snapshot location using export.
        let filter = roko_neuro::knowledge_store::ExportFilter::default();
        ensure_neuro_directory(
            snapshot
                .knowledge
                .parent()
                .ok_or_else(|| anyhow!("resolve backup directory"))?,
            "backup",
        )?;

        // Re-add just the top N entries through a temporary store.
        let temp_store = KnowledgeStore::new(snapshot.knowledge.clone());
        let _count = entries.len();
        if !entries.is_empty() {
            temp_store.ingest(entries)?;
        }

        // Also export using the JSONL export format for maximum compatibility.
        let exported_count = temp_store.export(&snapshot.knowledge, &filter)?;

        let confirmations_present = sync_optional_neuro_file(
            &live.confirmations,
            &snapshot.confirmations,
            force,
            "backup",
        )?;

        return Ok(NeuroTransferReport {
            live,
            snapshot,
            confirmations_present,
            entries_exported: exported_count,
            entries_restored: 0,
            entries_filtered: 0,
        });
    }

    let confirmations_present = sync_neuro_store_files(&live, &snapshot, force, "backup")?;

    // Count entries in the exported file.
    let entries_exported = if snapshot.knowledge.exists() {
        let store = KnowledgeStore::new(snapshot.knowledge.clone());
        store.read_all().map(|e| e.len()).unwrap_or(0)
    } else {
        0
    };

    Ok(NeuroTransferReport {
        live,
        snapshot,
        confirmations_present,
        entries_exported,
        entries_restored: 0,
        entries_filtered: 0,
    })
}


pub(crate) fn restore_neuro_store(
    workdir: &Path,
    source: &Path,
    force: bool,
    generation: u32,
    min_confidence: Option<f64>,
    type_filters: Option<&[String]>,
) -> Result<NeuroTransferReport> {
    let live = neuro_live_files(workdir);
    let snapshot = neuro_snapshot_files(source);

    // Apply confidence decay and filtering during restore.
    let confidence_multiplier = 0.85_f64.powi(generation as i32);

    // Read the source backup entries.
    let source_store = KnowledgeStore::new(snapshot.knowledge.clone());
    let source_entries = source_store
        .read_all()
        .with_context(|| format!("read backup entries from {}", snapshot.knowledge.display()))?;

    let total_source = source_entries.len();

    // Apply filters: type filter and min confidence.
    let filtered: Vec<_> = source_entries
        .into_iter()
        .filter(|entry| {
            if let Some(types) = type_filters {
                let kind_str = format!("{:?}", entry.kind).to_lowercase();
                types.iter().any(|t| kind_str.contains(&t.to_lowercase()))
            } else {
                true
            }
        })
        .filter(|entry| {
            if let Some(min) = min_confidence {
                entry.confidence >= min
            } else {
                true
            }
        })
        .map(|mut entry| {
            // Apply 0.85^N confidence decay.
            entry.confidence = (entry.confidence * confidence_multiplier).clamp(0.0, 1.0);
            // Reset to Transient tier (quarantine).
            entry.tier = roko_neuro::KnowledgeTier::Transient;
            // Mark source as restore with generation info.
            entry.source = Some(format!("restore:gen{generation}"));
            entry
        })
        .collect();

    let entries_restored = filtered.len();
    let entries_filtered = total_source.saturating_sub(entries_restored);

    // Write filtered entries to the live store.
    let dest_store = KnowledgeStore::for_workdir(workdir);
    if let Some(parent) = dest_store.path().parent() {
        std::fs::create_dir_all(parent)?;
    }

    // If force is not set and the live store exists, check before overwriting.
    if dest_store.path().exists() && !force {
        let existing = dest_store.read_all().unwrap_or_default();
        if !existing.is_empty() {
            bail!(
                "restore would modify existing knowledge store at {}. Re-run with --force to proceed.",
                dest_store.path().display()
            );
        }
    }

    if !filtered.is_empty() {
        dest_store.ingest(filtered)?;
    }

    // Copy confirmations if present.
    let confirmations_present = if snapshot.confirmations.exists() {
        sync_optional_neuro_file(
            &snapshot.confirmations,
            &live.confirmations,
            force,
            "restore",
        )?
    } else {
        false
    };

    Ok(NeuroTransferReport {
        live,
        snapshot,
        confirmations_present,
        entries_exported: 0,
        entries_restored,
        entries_filtered,
    })
}


pub(crate) fn neuro_live_files(workdir: &Path) -> NeuroFileSet {
    let store = KnowledgeStore::for_workdir(workdir);
    NeuroFileSet {
        knowledge: store.path().to_path_buf(),
        confirmations: store.confirmations_path().to_path_buf(),
    }
}


pub(crate) fn neuro_snapshot_files(root: &Path) -> NeuroFileSet {
    NeuroFileSet {
        knowledge: root.join(NEURO_KNOWLEDGE_FILE),
        confirmations: root.join(NEURO_CONFIRMATIONS_FILE),
    }
}


pub(crate) fn sync_neuro_store_files(
    source: &NeuroFileSet,
    destination: &NeuroFileSet,
    force: bool,
    operation: &str,
) -> Result<bool> {
    let destination_root = destination
        .knowledge
        .parent()
        .ok_or_else(|| anyhow!("resolve {operation} destination directory"))?;
    ensure_neuro_directory(destination_root, operation)?;

    copy_neuro_file(&source.knowledge, &destination.knowledge, force, operation)?;
    sync_optional_neuro_file(
        &source.confirmations,
        &destination.confirmations,
        force,
        operation,
    )
}


pub(crate) fn ensure_neuro_directory(path: &Path, operation: &str) -> Result<()> {
    if path.exists() && !path.is_dir() {
        bail!(
            "{operation} target must be a directory, found file at {}",
            path.display()
        );
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("create {operation} directory {}", path.display()))?;
    Ok(())
}


pub(crate) fn copy_neuro_file(source: &Path, destination: &Path, force: bool, operation: &str) -> Result<()> {
    if !source.exists() {
        bail!("{operation} source file not found: {}", source.display());
    }
    if destination.exists() && !force {
        bail!(
            "{operation} would overwrite {}. Re-run with --force to replace it.",
            destination.display()
        );
    }
    std::fs::copy(source, destination).with_context(|| {
        format!(
            "{operation} {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}


pub(crate) fn sync_optional_neuro_file(
    source: &Path,
    destination: &Path,
    force: bool,
    operation: &str,
) -> Result<bool> {
    if source.exists() {
        copy_neuro_file(source, destination, force, operation)?;
        return Ok(true);
    }

    if destination.exists() {
        if !force {
            bail!(
                "{operation} would leave stale optional file at {}. Re-run with --force to replace it.",
                destination.display()
            );
        }
        std::fs::remove_file(destination).with_context(|| {
            format!(
                "{operation} remove stale optional file {}",
                destination.display()
            )
        })?;
    }

    Ok(false)
}


pub(crate) const NEURO_KNOWLEDGE_FILE: &str = "knowledge.jsonl";
pub(crate) const NEURO_CONFIRMATIONS_FILE: &str = "knowledge-confirmations.jsonl";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeuroFileSet {
    pub(crate) knowledge: PathBuf,
    pub(crate) confirmations: PathBuf,
}


