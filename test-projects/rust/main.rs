use std::collections::{BTreeMap, BTreeSet};

fn normalize_project_code(project_code: &str) -> String {
    project_code.trim().to_ascii_uppercase()
}

fn build_rows() -> Vec<(i32, String, i32)> {
    vec![
        (101, "vivi".to_string(), 11),
        (202, "vivi".to_string(), 17),
        (303, "nova".to_string(), 13),
        (404, "vivi".to_string(), 23),
        (505, "nova".to_string(), 19),
    ]
}

fn score_multiplier(project_code: &str) -> i32 {
    match project_code {
        "VIVI" => 3,
        "NOVA" => 2,
        _ => 1,
    }
}

fn validate_rows(rows: &[(i32, String, i32)]) {
    let mut seen_user_ids = BTreeSet::new();
    for (user_id, _project_code, score) in rows {
        assert!(seen_user_ids.insert(*user_id), "duplicate user id");
        assert!(*score > 0, "invalid score");
    }
}

fn aggregate_scores(rows: &[(i32, String, i32)]) -> BTreeMap<String, i32> {
    let mut project_totals = BTreeMap::new();
    for (_user_id, project_code, score) in rows {
        let normalized = normalize_project_code(project_code);
        let weighted_score = score * score_multiplier(&normalized);
        *project_totals.entry(normalized).or_insert(0) += weighted_score;
    }
    project_totals
}

fn find_priority_users(rows: &[(i32, String, i32)], threshold: i32) -> Vec<i32> {
    let mut priority_user_ids = Vec::new();
    for (user_id, project_code, score) in rows {
        let normalized = normalize_project_code(project_code);
        let weighted_score = score * score_multiplier(&normalized);
        if weighted_score >= threshold {
            priority_user_ids.push(*user_id);
        }
    }
    priority_user_ids.sort();
    priority_user_ids
}

fn project_average_scores(rows: &[(i32, String, i32)]) -> BTreeMap<String, f64> {
    let mut totals: BTreeMap<String, i32> = BTreeMap::new();
    let mut counts: BTreeMap<String, i32> = BTreeMap::new();
    for (_user_id, project_code, score) in rows {
        let normalized = normalize_project_code(project_code);
        *totals.entry(normalized.clone()).or_insert(0) += score;
        *counts.entry(normalized).or_insert(0) += 1;
    }

    let mut averages = BTreeMap::new();
    for (project, total) in totals {
        let count = counts.get(&project).copied().unwrap_or(1);
        averages.insert(project, total as f64 / count as f64);
    }
    averages
}

fn project_signature(project_totals: &BTreeMap<String, i32>, priority_user_ids: &[i32]) -> String {
    let totals_part = project_totals
        .iter()
        .map(|(project, total)| format!("{}:{}", project, total))
        .collect::<Vec<_>>()
        .join(";");
    let users_part = priority_user_ids
        .iter()
        .map(|user_id| user_id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("{}|{}", totals_part, users_part)
}

fn explain_summary(project_totals: &BTreeMap<String, i32>, averages: &BTreeMap<String, f64>) -> String {
    project_totals
        .iter()
        .map(|(project, total)| {
            let avg = averages.get(project).copied().unwrap_or_default();
            format!("{}[total={},avg={:.1}]", project, total, avg)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let rows = build_rows();
    validate_rows(&rows);
    let project_totals = aggregate_scores(&rows);
    let priority_user_ids = find_priority_users(&rows, 40);
    let averages = project_average_scores(&rows);
    let signature = project_signature(&project_totals, &priority_user_ids);
    let summary = explain_summary(&project_totals, &averages);
    println!("{}", signature);
    println!("{}", summary);
}
