#include <algorithm>
#include <iostream>
#include <map>
#include <string>
#include <tuple>
#include <vector>

struct UserProfile {
    int user_id;
    std::string project_code;
    int score;
};

std::string normalize_project_code(const std::string& project_code) {
    std::string normalized = project_code;
    std::transform(normalized.begin(), normalized.end(), normalized.begin(), ::toupper);
    return normalized;
}

std::vector<UserProfile> build_profiles() {
    std::vector<std::tuple<int, std::string, int>> source_rows = {
        {101, "vivi", 11},
        {202, "vivi", 17},
        {303, "nova", 13},
        {404, "vivi", 23},
        {505, "nova", 19},
    };

    std::vector<UserProfile> profiles;
    for (const auto& row : source_rows) {
        profiles.push_back(UserProfile{
            std::get<0>(row),
            normalize_project_code(std::get<1>(row)),
            std::get<2>(row),
        });
    }
    return profiles;
}

int score_multiplier(const std::string& project_code) {
    if (project_code == "VIVI") {
        return 3;
    }
    if (project_code == "NOVA") {
        return 2;
    }
    return 1;
}

std::map<std::string, int> aggregate_scores(const std::vector<UserProfile>& profiles) {
    std::map<std::string, int> project_totals;
    for (const auto& profile : profiles) {
        int weighted_score = profile.score * score_multiplier(profile.project_code);
        project_totals[profile.project_code] += weighted_score;
    }
    return project_totals;
}

std::vector<int> find_priority_users(const std::vector<UserProfile>& profiles, int threshold) {
    std::vector<int> priority_user_ids;
    for (const auto& profile : profiles) {
        int weighted_score = profile.score * score_multiplier(profile.project_code);
        if (weighted_score >= threshold) {
            priority_user_ids.push_back(profile.user_id);
        }
    }
    std::sort(priority_user_ids.begin(), priority_user_ids.end());
    return priority_user_ids;
}

std::string project_signature(const std::map<std::string, int>& project_totals,
                              const std::vector<int>& priority_user_ids) {
    std::string totals_part;
    bool first_total = true;
    for (const auto& [project, total] : project_totals) {
        if (!first_total) {
            totals_part += ';';
        }
        first_total = false;
        totals_part += project + ":" + std::to_string(total);
    }

    std::string users_part;
    for (size_t i = 0; i < priority_user_ids.size(); ++i) {
        if (i > 0) {
            users_part += ',';
        }
        users_part += std::to_string(priority_user_ids[i]);
    }
    return totals_part + "|" + users_part;
}

int main() {
    std::vector<UserProfile> profiles = build_profiles();
    std::map<std::string, int> project_totals = aggregate_scores(profiles);
    std::vector<int> priority_user_ids = find_priority_users(profiles, 40);
    std::string signature = project_signature(project_totals, priority_user_ids);
    std::cout << signature << std::endl;
    return 0;
}
