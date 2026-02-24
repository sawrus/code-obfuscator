using System;
using System.Collections.Generic;

internal record UserProfile(int UserId, string ProjectCode, int Score);

internal static class Program
{
    private static string NormalizeProjectCode(string projectCode)
    {
        return projectCode.Trim().ToUpperInvariant();
    }

    private static List<UserProfile> BuildProfiles()
    {
        var sourceRows = new (int UserId, string ProjectCode, int Score)[]
        {
            (101, "vivi", 11),
            (202, "vivi", 17),
            (303, "nova", 13),
            (404, "vivi", 23),
            (505, "nova", 19),
        };

        var profiles = new List<UserProfile>();
        foreach (var row in sourceRows)
        {
            profiles.Add(new UserProfile(row.UserId, NormalizeProjectCode(row.ProjectCode), row.Score));
        }

        return profiles;
    }

    private static int ScoreMultiplier(string projectCode)
    {
        if (projectCode == "VIVI")
        {
            return 3;
        }

        if (projectCode == "NOVA")
        {
            return 2;
        }

        return 1;
    }

    private static Dictionary<string, int> AggregateScores(List<UserProfile> profiles)
    {
        var projectTotals = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (var profile in profiles)
        {
            var weightedScore = profile.Score * ScoreMultiplier(profile.ProjectCode);
            projectTotals.TryGetValue(profile.ProjectCode, out var current);
            projectTotals[profile.ProjectCode] = current + weightedScore;
        }

        return projectTotals;
    }

    private static List<int> FindPriorityUsers(List<UserProfile> profiles, int threshold)
    {
        var priorityUserIds = new List<int>();
        foreach (var profile in profiles)
        {
            var weightedScore = profile.Score * ScoreMultiplier(profile.ProjectCode);
            if (weightedScore >= threshold)
            {
                priorityUserIds.Add(profile.UserId);
            }
        }

        priorityUserIds.Sort();
        return priorityUserIds;
    }

    private static string ProjectSignature(Dictionary<string, int> projectTotals, List<int> priorityUserIds)
    {
        var projects = new List<string>(projectTotals.Keys);
        projects.Sort(StringComparer.Ordinal);
        var totalsPart = string.Join(';', projects.ConvertAll(project => $"{project}:{projectTotals[project]}"));
        var usersPart = string.Join(',', priorityUserIds);
        return $"{totalsPart}|{usersPart}";
    }


    private static void ValidateProfiles(List<UserProfile> profiles)
    {
        var seen = new HashSet<int>();
        foreach (var profile in profiles)
        {
            if (!seen.Add(profile.UserId))
            {
                throw new InvalidOperationException($"duplicate user id: {profile.UserId}");
            }

            if (profile.Score <= 0)
            {
                throw new InvalidOperationException($"invalid score: {profile.Score}");
            }
        }
    }

    private static Dictionary<string, double> ProjectAverageScores(List<UserProfile> profiles)
    {
        var totals = new Dictionary<string, int>(StringComparer.Ordinal);
        var counts = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (var profile in profiles)
        {
            totals.TryGetValue(profile.ProjectCode, out var total);
            counts.TryGetValue(profile.ProjectCode, out var count);
            totals[profile.ProjectCode] = total + profile.Score;
            counts[profile.ProjectCode] = count + 1;
        }

        var averages = new Dictionary<string, double>(StringComparer.Ordinal);
        foreach (var pair in totals)
        {
            averages[pair.Key] = (double)pair.Value / counts[pair.Key];
        }
        return averages;
    }

    private static void Main()
    {
        var profiles = BuildProfiles();
        ValidateProfiles(profiles);
        var projectTotals = AggregateScores(profiles);
        var priorityUserIds = FindPriorityUsers(profiles, 40);
        var averages = ProjectAverageScores(profiles);
        var signature = ProjectSignature(projectTotals, priorityUserIds);
        Console.WriteLine(signature);
        Console.WriteLine($"avg_count={averages.Count}");
    }
}
