use super::common::{location_at, observation};
use crate::{evidence::Observation, input::Input};
use serde_json::json;
use std::{collections::BTreeSet, path::Path};

fn responsibility(path: &str) -> String {
    let path = Path::new(path);
    let components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components.len() > 1 {
        components[0].into()
    } else {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(path.as_os_str().to_str().unwrap_or("unowned"))
            .into()
    }
}

pub(super) fn divergent_change(input: &Input) -> Vec<Observation> {
    if !input.history.available {
        return Vec::new();
    }
    input
        .files
        .keys()
        .filter_map(|path| {
            let touching = input
                .history
                .commits
                .iter()
                .filter(|commit| commit.files.contains(path))
                .collect::<Vec<_>>();
            if touching.is_empty() {
                return None;
            }
            let responsibilities = touching
                .iter()
                .flat_map(|commit| commit.files.iter())
                .filter(|changed| *changed != path)
                .map(|changed| responsibility(changed))
                .collect::<BTreeSet<_>>();
            let source = &input.files[path];
            Some(observation(
                format!("{path}::git-change-profile"),
                location_at(path, source, 0),
                [
                    ("changes", touching.len() as u64),
                    ("responsibilities", responsibilities.len() as u64),
                ],
                json!({
                    "collector": "git_cochange_responsibilities_v1",
                    "history_limit": 200,
                    "commit_ids": touching.iter().map(|commit| &commit.id).collect::<Vec<_>>(),
                    "responsibilities": responsibilities,
                }),
            ))
        })
        .collect()
}

pub(super) fn shotgun_surgery(input: &Input) -> Vec<Observation> {
    if !input.history.available {
        return Vec::new();
    }
    input
        .history
        .commits
        .iter()
        .filter_map(|commit| {
            let owners = commit
                .files
                .iter()
                .map(|path| responsibility(path))
                .collect::<BTreeSet<_>>();
            let anchor = commit.files.first()?;
            let source = input.files.get(anchor)?;
            Some(observation(
                format!("git:{}", commit.id),
                location_at(anchor, source, 0),
                [("owners", owners.len() as u64)],
                json!({
                    "collector": "git_commit_owner_fanout_v1",
                    "history_limit": 200,
                    "commit_id": commit.id,
                    "owners": owners,
                    "changed_files": commit.files,
                }),
            ))
        })
        .collect()
}
