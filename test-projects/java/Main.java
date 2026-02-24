import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class Main {
    static class UserProfile {
        final int userId;
        final String projectCode;
        final int score;

        UserProfile(int userId, String projectCode, int score) {
            this.userId = userId;
            this.projectCode = projectCode;
            this.score = score;
        }
    }

    static String normalizeProjectCode(String projectCode) {
        return projectCode.trim().toUpperCase();
    }

    static List<UserProfile> buildProfiles() {
        int[][] sourceRows = {
                {101, 11},
                {202, 17},
                {303, 13},
                {404, 23},
                {505, 19}
        };
        String[] projects = {"vivi", "vivi", "nova", "vivi", "nova"};
        List<UserProfile> profiles = new ArrayList<>();
        for (int i = 0; i < sourceRows.length; i++) {
            profiles.add(new UserProfile(sourceRows[i][0], normalizeProjectCode(projects[i]), sourceRows[i][1]));
        }
        return profiles;
    }

    static int scoreMultiplier(String projectCode) {
        if ("VIVI".equals(projectCode)) {
            return 3;
        }
        if ("NOVA".equals(projectCode)) {
            return 2;
        }
        return 1;
    }

    static Map<String, Integer> aggregateScores(List<UserProfile> profiles) {
        Map<String, Integer> projectTotals = new HashMap<>();
        for (UserProfile profile : profiles) {
            int weightedScore = profile.score * scoreMultiplier(profile.projectCode);
            int current = projectTotals.getOrDefault(profile.projectCode, 0);
            projectTotals.put(profile.projectCode, current + weightedScore);
        }
        return projectTotals;
    }

    static List<Integer> findPriorityUsers(List<UserProfile> profiles, int threshold) {
        List<Integer> priorityUserIds = new ArrayList<>();
        for (UserProfile profile : profiles) {
            int weightedScore = profile.score * scoreMultiplier(profile.projectCode);
            if (weightedScore >= threshold) {
                priorityUserIds.add(profile.userId);
            }
        }
        Collections.sort(priorityUserIds);
        return priorityUserIds;
    }

    static String projectSignature(Map<String, Integer> projectTotals, List<Integer> priorityUserIds) {
        List<String> projects = new ArrayList<>(projectTotals.keySet());
        Collections.sort(projects);
        StringBuilder totalsPart = new StringBuilder();
        for (int i = 0; i < projects.size(); i++) {
            if (i > 0) {
                totalsPart.append(';');
            }
            String project = projects.get(i);
            totalsPart.append(project).append(':').append(projectTotals.get(project));
        }

        StringBuilder usersPart = new StringBuilder();
        for (int i = 0; i < priorityUserIds.size(); i++) {
            if (i > 0) {
                usersPart.append(',');
            }
            usersPart.append(priorityUserIds.get(i));
        }
        return totalsPart + "|" + usersPart;
    }

    public static void main(String[] args) {
        List<UserProfile> profiles = buildProfiles();
        Map<String, Integer> projectTotals = aggregateScores(profiles);
        List<Integer> priorityUserIds = findPriorityUsers(profiles, 40);
        String signature = projectSignature(projectTotals, priorityUserIds);
        System.out.println(signature);
    }
}
