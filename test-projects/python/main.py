from typing import Dict, List, Tuple


def normalize_project_code(project_code: str) -> str:
    return project_code.strip().upper()


def build_rows() -> List[Tuple[int, str, int]]:
    return [
        (101, "vivi", 11),
        (202, "vivi", 17),
        (303, "nova", 13),
        (404, "vivi", 23),
        (505, "nova", 19),
    ]


def score_multiplier(project_code: str) -> int:
    if project_code == "VIVI":
        return 3
    if project_code == "NOVA":
        return 2
    return 1


def validate_rows(rows: List[Tuple[int, str, int]]) -> None:
    seen_user_ids = set()
    for user_id, _project_code, score in rows:
        if user_id in seen_user_ids:
            raise ValueError(f"duplicate user id: {user_id}")
        seen_user_ids.add(user_id)
        if score <= 0:
            raise ValueError(f"invalid score: {score}")


def aggregate_scores(rows: List[Tuple[int, str, int]]) -> Dict[str, int]:
    project_totals: Dict[str, int] = {}
    for _user_id, project_code, score in rows:
        normalized = normalize_project_code(project_code)
        weighted_score = score * score_multiplier(normalized)
        project_totals[normalized] = project_totals.get(normalized, 0) + weighted_score
    return project_totals


def find_priority_users(rows: List[Tuple[int, str, int]], threshold: int) -> List[int]:
    priority_user_ids: List[int] = []
    for user_id, project_code, score in rows:
        normalized = normalize_project_code(project_code)
        weighted_score = score * score_multiplier(normalized)
        if weighted_score >= threshold:
            priority_user_ids.append(user_id)
    return sorted(priority_user_ids)


def project_average_scores(rows: List[Tuple[int, str, int]]) -> Dict[str, float]:
    totals: Dict[str, int] = {}
    counts: Dict[str, int] = {}
    for _user_id, project_code, score in rows:
        normalized = normalize_project_code(project_code)
        totals[normalized] = totals.get(normalized, 0) + score
        counts[normalized] = counts.get(normalized, 0) + 1
    return {project: totals[project] / counts[project] for project in totals}


def project_signature(project_totals: Dict[str, int], priority_user_ids: List[int]) -> str:
    ordered_totals = sorted(project_totals.items())
    totals_part = ";".join(f"{project}:{total}" for project, total in ordered_totals)
    users_part = ",".join(str(user_id) for user_id in priority_user_ids)
    return f"{totals_part}|{users_part}"


def explain_summary(project_totals: Dict[str, int], averages: Dict[str, float]) -> str:
    projects = sorted(project_totals)
    pieces = []
    for project in projects:
        pieces.append(f"{project}[total={project_totals[project]},avg={averages[project]:.1f}]")
    return " ".join(pieces)


def main() -> None:
    rows = build_rows()
    validate_rows(rows)
    project_totals = aggregate_scores(rows)
    priority_user_ids = find_priority_users(rows, threshold=40)
    averages = project_average_scores(rows)
    signature = project_signature(project_totals, priority_user_ids)
    summary = explain_summary(project_totals, averages)
    print(signature)
    print(summary)


if __name__ == "__main__":
    main()
