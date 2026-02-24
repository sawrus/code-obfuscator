#!/usr/bin/env bash
set -euo pipefail

normalize_project_code() {
  local project_code="$1"
  echo "${project_code^^}"
}

score_multiplier() {
  local project_code="$1"
  if [[ "$project_code" == "VIVI" ]]; then
    echo 3
  elif [[ "$project_code" == "NOVA" ]]; then
    echo 2
  else
    echo 1
  fi
}

build_profiles() {
  cat <<'DATA'
101,vivi,11
202,vivi,17
303,nova,13
404,vivi,23
505,nova,19
DATA
}

declare -A project_totals=()
declare -a priority_user_ids=()

aggregate_scores() {
  while IFS=',' read -r user_id project_code score; do
    local normalized_project
    normalized_project="$(normalize_project_code "$project_code")"
    local multiplier
    multiplier="$(score_multiplier "$normalized_project")"
    local weighted_score=$((score * multiplier))
    local current_total=${project_totals[$normalized_project]:-0}
    project_totals[$normalized_project]=$((current_total + weighted_score))
    if (( weighted_score >= 40 )); then
      priority_user_ids+=("$user_id")
    fi
  done < <(build_profiles)
}

sorted_keys() {
  printf '%s
' "${!project_totals[@]}" | sort
}

join_by() {
  local delimiter="$1"
  shift
  local first=1
  for part in "$@"; do
    if (( first )); then
      printf '%s' "$part"
      first=0
    else
      printf '%s%s' "$delimiter" "$part"
    fi
  done
}

project_signature() {
  local -a totals_parts=()
  while IFS= read -r project; do
    totals_parts+=("$project:${project_totals[$project]}")
  done < <(sorted_keys)

  local -a sorted_users=()
  mapfile -t sorted_users < <(printf '%s
' "${priority_user_ids[@]}" | sort -n)

  local totals_part users_part
  totals_part="$(join_by ';' "${totals_parts[@]}")"
  users_part="$(join_by ',' "${sorted_users[@]}")"
  echo "${totals_part}|${users_part}"
}


validate_profiles() {
  local -A seen=()
  while IFS=',' read -r user_id project_code score; do
    if [[ -n "${seen[$user_id]:-}" ]]; then
      echo "duplicate user id: $user_id" >&2
      exit 1
    fi
    seen[$user_id]=1
    if (( score <= 0 )); then
      echo "invalid score: $score" >&2
      exit 1
    fi
  done < <(build_profiles)
}

project_average_scores() {
  declare -A totals=()
  declare -A counts=()
  while IFS=',' read -r _user_id project_code score; do
    local normalized_project
    normalized_project="$(normalize_project_code "$project_code")"
    totals[$normalized_project]=$(( ${totals[$normalized_project]:-0} + score ))
    counts[$normalized_project]=$(( ${counts[$normalized_project]:-0} + 1 ))
  done < <(build_profiles)

  local project
  for project in "${!totals[@]}"; do
    local avg=$(( totals[$project] / counts[$project] ))
    echo "$project:$avg"
  done | sort
}

main() {
  validate_profiles
  aggregate_scores
  project_signature
  project_average_scores >/dev/null
}

main
