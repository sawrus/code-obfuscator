function normalizeProjectCode(projectCode) {
  return projectCode.trim().toUpperCase();
}

function buildProfiles() {
  const sourceRows = [
    [101, "vivi", 11],
    [202, "vivi", 17],
    [303, "nova", 13],
    [404, "vivi", 23],
    [505, "nova", 19],
  ];
  const profiles = [];
  for (const [userId, projectCode, score] of sourceRows) {
    profiles.push({
      userId,
      projectCode: normalizeProjectCode(projectCode),
      score,
    });
  }
  return profiles;
}

function scoreMultiplier(projectCode) {
  if (projectCode === "VIVI") {
    return 3;
  }
  if (projectCode === "NOVA") {
    return 2;
  }
  return 1;
}

function aggregateScores(profiles) {
  const projectTotals = new Map();
  for (const profile of profiles) {
    const weightedScore = profile.score * scoreMultiplier(profile.projectCode);
    const current = projectTotals.get(profile.projectCode) || 0;
    projectTotals.set(profile.projectCode, current + weightedScore);
  }
  return projectTotals;
}

function findPriorityUsers(profiles, threshold) {
  const priorityUserIds = [];
  for (const profile of profiles) {
    const weightedScore = profile.score * scoreMultiplier(profile.projectCode);
    if (weightedScore >= threshold) {
      priorityUserIds.push(profile.userId);
    }
  }
  return priorityUserIds.sort((left, right) => left - right);
}

function projectSignature(projectTotals, priorityUserIds) {
  const orderedTotals = [...projectTotals.entries()].sort(([left], [right]) =>
    left.localeCompare(right)
  );
  const totalsPart = orderedTotals
    .map(([project, total]) => `${project}:${total}`)
    .join(";");
  const usersPart = priorityUserIds.map((userId) => String(userId)).join(",");
  return `${totalsPart}|${usersPart}`;
}

function validateProfiles(profiles) {
  const seenUserIds = new Set();
  for (const profile of profiles) {
    if (seenUserIds.has(profile.userId)) {
      throw new Error(`duplicate user id: ${profile.userId}`);
    }
    seenUserIds.add(profile.userId);
    if (profile.score <= 0) {
      throw new Error(`invalid score: ${profile.score}`);
    }
  }
}

function projectAverageScores(profiles) {
  const totals = new Map();
  const counts = new Map();
  for (const profile of profiles) {
    totals.set(profile.projectCode, (totals.get(profile.projectCode) || 0) + profile.score);
    counts.set(profile.projectCode, (counts.get(profile.projectCode) || 0) + 1);
  }
  const averages = new Map();
  for (const [project, total] of totals.entries()) {
    averages.set(project, total / (counts.get(project) || 1));
  }
  return averages;
}

function explainSummary(projectTotals, averages) {
  const projects = [...projectTotals.keys()].sort((left, right) =>
    left.localeCompare(right)
  );
  return projects
    .map((project) => `${project}[total=${projectTotals.get(project)},avg=${averages.get(project)?.toFixed(1)}]`)
    .join(" ");
}


function main() {
  const profiles = buildProfiles();
  validateProfiles(profiles);
  const projectTotals = aggregateScores(profiles);
  const priorityUserIds = findPriorityUsers(profiles, 40);
  const averages = projectAverageScores(profiles);
  const summary = explainSummary(projectTotals, averages);
  const signature = projectSignature(projectTotals, priorityUserIds);
  console.log(signature);
  console.log(summary);
}

main();
