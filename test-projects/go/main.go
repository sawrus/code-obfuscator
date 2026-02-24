package main

import (
	"fmt"
	"sort"
	"strings"
)

type Row struct {
	UserID      int
	ProjectCode string
	Score       int
}

func normalizeProjectCode(projectCode string) string { return strings.ToUpper(strings.TrimSpace(projectCode)) }

func buildRows() []Row {
	return []Row{{101, "vivi", 11}, {202, "vivi", 17}, {303, "nova", 13}, {404, "vivi", 23}, {505, "nova", 19}}
}

func scoreMultiplier(projectCode string) int {
	if projectCode == "VIVI" { return 3 }
	if projectCode == "NOVA" { return 2 }
	return 1
}

func validateRows(rows []Row) {
	seen := map[int]struct{}{}
	for _, row := range rows {
		if _, ok := seen[row.UserID]; ok { panic("duplicate user id") }
		seen[row.UserID] = struct{}{}
		if row.Score <= 0 { panic("invalid score") }
	}
}

func aggregateScores(rows []Row) map[string]int {
	projectTotals := map[string]int{}
	for _, row := range rows {
		normalized := normalizeProjectCode(row.ProjectCode)
		weighted := row.Score * scoreMultiplier(normalized)
		projectTotals[normalized] += weighted
	}
	return projectTotals
}

func findPriorityUsers(rows []Row, threshold int) []int {
	priority := make([]int, 0)
	for _, row := range rows {
		normalized := normalizeProjectCode(row.ProjectCode)
		weighted := row.Score * scoreMultiplier(normalized)
		if weighted >= threshold { priority = append(priority, row.UserID) }
	}
	sort.Ints(priority)
	return priority
}

func projectAverageScores(rows []Row) map[string]float64 {
	totals := map[string]int{}
	counts := map[string]int{}
	for _, row := range rows {
		normalized := normalizeProjectCode(row.ProjectCode)
		totals[normalized] += row.Score
		counts[normalized]++
	}
	averages := map[string]float64{}
	for project, total := range totals { averages[project] = float64(total) / float64(counts[project]) }
	return averages
}

func projectSignature(projectTotals map[string]int, priority []int) string {
	projects := make([]string, 0, len(projectTotals))
	for project := range projectTotals { projects = append(projects, project) }
	sort.Strings(projects)
	totals := make([]string, 0, len(projects))
	for _, project := range projects { totals = append(totals, fmt.Sprintf("%s:%d", project, projectTotals[project])) }
	users := make([]string, 0, len(priority))
	for _, userID := range priority { users = append(users, fmt.Sprintf("%d", userID)) }
	return strings.Join(totals, ";") + "|" + strings.Join(users, ",")
}

func explainSummary(projectTotals map[string]int, averages map[string]float64) string {
	projects := make([]string, 0, len(projectTotals))
	for project := range projectTotals { projects = append(projects, project) }
	sort.Strings(projects)
	parts := make([]string, 0, len(projects))
	for _, project := range projects {
		parts = append(parts, fmt.Sprintf("%s[total=%d,avg=%.1f]", project, projectTotals[project], averages[project]))
	}
	return strings.Join(parts, " ")
}

func main() {
	rows := buildRows()
	validateRows(rows)
	projectTotals := aggregateScores(rows)
	priority := findPriorityUsers(rows, 40)
	averages := projectAverageScores(rows)
	fmt.Println(projectSignature(projectTotals, priority))
	fmt.Println(explainSummary(projectTotals, averages))
}
