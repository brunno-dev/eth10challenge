//! Host-only profiling with a frozen history directory; never derives wallets
//! or writes negative evidence. Generation and membership are timed separately.
#![allow(dead_code, unused_imports)]
#[path = "../src/candidates.rs"]
mod candidates;
#[path = "../src/checkpoint.rs"]
mod checkpoint;
#[path = "../src/eth.rs"]
mod eth;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/local_history.rs"]
mod local_history;
#[path = "../src/metrics.rs"]
mod metrics;
#[path = "../src/plan.rs"]
mod plan;
#[path = "../src/pruning.rs"]
mod pruning;

use anyhow::Result;
use bip39::Language;
use bitcoin::hashes::{sha256, Hash};
use candidates::Slot;
use clap::Parser;
use std::{hint::black_box, path::PathBuf, time::Instant};

// Root helpers referenced by the included production modules. This benchmark
// intentionally accepts only the English puzzle history.
fn parse_language(language: &str) -> Result<Language> {
    anyhow::ensure!(
        language.eq_ignore_ascii_case("english"),
        "English history required"
    );
    Ok(Language::English)
}
fn format_u128(n: u128) -> String {
    n.to_string()
}
fn describe_search(_: &[Slot; 12], _: &[u16], _: &[u16], _: &[&str]) -> Result<()> {
    Ok(())
}

#[derive(Parser)]
struct Args {
    #[arg(long)]
    history: PathBuf,
    #[arg(long, default_value_t = 1_000_000)]
    limit: usize,
    #[arg(long, default_value_t = 3)]
    trials: usize,
    #[arg(long, default_value = "coins", value_parser = ["coins", "combined"])]
    case: String,
    // Cargo passes this to custom benchmark executables.
    #[arg(long, hide = true)]
    bench: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let index = |s: &str| Language::English.word_list().binary_search(&s).unwrap() as u16;
    let mut slots = [Slot::Hole; 12];
    for (i, w) in [(0, "dutch"), (4, "fog"), (11, "parrot")] {
        slots[i] = Slot::Fixed(index(w));
    }
    let mut video = ["fiber", "wood", "winter", "rib"].map(index).to_vec();
    if args.case == "coins" {
        video.extend(["atom", "link", "basic", "token", "dash"].map(index));
    } else {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../history/research/video-onscreen-words.json"
        ))?;
        let mut words = ["onscreen_not_in_pool", "spoken_not_in_pool"]
            .into_iter()
            .flat_map(|key| data[key].as_array().unwrap())
            .map(|word| word.as_str().unwrap())
            .collect::<Vec<_>>();
        words.sort_unstable();
        words.dedup();
        for word in words {
            let id = index(word);
            if !video.contains(&id) && ![index("fog"), index("parrot")].contains(&id) {
                video.push(id);
            }
        }
    }
    let plan = plan::Plan::Batches {
        slots,
        post: ["fiber", "fork", "dinner", "cloud", "live"]
            .map(index)
            .to_vec(),
        video,
        post_open: 5,
    };
    plan.validate()?;
    let target = eth::parse_address("0x9c2f44efad0c1e852a09df9939e6daf061140caf")?;
    let mut paths = std::fs::read_dir(&args.history)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|ext| ext == "json"));
    paths.sort();
    let mut compatible = Vec::new();
    for path in paths {
        let record = local_history::Record::load(&path)?;
        if record.is_compatible(Language::English, &target, false) {
            compatible.push(path);
        }
    }
    let paths = compatible;
    let rule = history::Exclusions::load(true, &paths)?;
    rule.compatible(Language::English, &target, false)?;
    for trial in 0..=args.trials {
        let start = Instant::now();
        let phrases: Vec<_> = plan.stream().take(args.limit).collect();
        let generation_seconds = start.elapsed().as_secs_f64();
        let start = Instant::now();
        let flags: Vec<u8> = phrases
            .iter()
            .map(|p| u8::from(black_box(&rule).contains(p)))
            .collect();
        let history_seconds = start.elapsed().as_secs_f64();
        if trial > 0 {
            println!(
                "{}",
                serde_json::json!({
                    "case": args.case, "trial": trial, "records": paths.len(),
                    "candidates": phrases.len(), "total": plan.count()?.to_string(),
                    "generation_seconds": generation_seconds, "history_seconds": history_seconds,
                    "excluded": flags.iter().map(|&x| usize::from(x)).sum::<usize>(),
                    "membership_sha256": sha256::Hash::hash(&flags).to_string(),
                    "history_fingerprint": rule.fingerprint()?,
                })
            );
        }
    }
    Ok(())
}
