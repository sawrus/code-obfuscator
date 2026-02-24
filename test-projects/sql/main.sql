BEGIN TRANSACTION;

CREATE TABLE project_user (
    user_id INTEGER PRIMARY KEY,
    project_code TEXT NOT NULL,
    score INTEGER NOT NULL
);

CREATE TABLE project_weight (
    project_code TEXT PRIMARY KEY,
    multiplier INTEGER NOT NULL
);

INSERT INTO project_user (user_id, project_code, score) VALUES
    (101, 'VIVI', 11),
    (202, 'VIVI', 17),
    (303, 'NOVA', 13),
    (404, 'VIVI', 23),
    (505, 'NOVA', 19);

INSERT INTO project_weight (project_code, multiplier) VALUES
    ('VIVI', 3),
    ('NOVA', 2);

WITH weighted AS (
    SELECT
        u.user_id,
        u.project_code,
        u.score,
        w.multiplier,
        u.score * w.multiplier AS weighted_score
    FROM project_user u
    JOIN project_weight w ON u.project_code = w.project_code
),
project_totals AS (
    SELECT
        project_code,
        SUM(weighted_score) AS total_weighted_score
    FROM weighted
    GROUP BY project_code
),
priority_users AS (
    SELECT
        user_id,
        project_code,
        weighted_score
    FROM weighted
    WHERE weighted_score >= 40
)
SELECT
    p.project_code,
    p.total_weighted_score,
    COALESCE(
        (
            SELECT GROUP_CONCAT(user_id, ',')
            FROM (
                SELECT user_id
                FROM priority_users pu
                WHERE pu.project_code = p.project_code
                ORDER BY user_id
            )
        ),
        ''
    ) AS priority_user_ids
FROM project_totals p
ORDER BY p.project_code;

CREATE VIEW project_summary AS
SELECT
    p.project_code,
    p.total_weighted_score,
    ROUND(AVG(w.score), 1) AS average_raw_score
FROM (
    SELECT
        u.project_code,
        SUM(u.score * w.multiplier) AS total_weighted_score
    FROM project_user u
    JOIN project_weight w ON u.project_code = w.project_code
    GROUP BY u.project_code
) p
JOIN project_user w ON p.project_code = w.project_code
GROUP BY p.project_code, p.total_weighted_score;

SELECT project_code, total_weighted_score, average_raw_score
FROM project_summary
ORDER BY project_code;

CREATE TEMP TABLE project_audit_log (
    audit_id INTEGER PRIMARY KEY,
    project_code TEXT NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value TEXT NOT NULL
);

INSERT INTO project_audit_log (project_code, metric_name, metric_value)
SELECT project_code, 'total_weighted_score', CAST(total_weighted_score AS TEXT)
FROM project_summary;

INSERT INTO project_audit_log (project_code, metric_name, metric_value)
SELECT project_code, 'average_raw_score', CAST(average_raw_score AS TEXT)
FROM project_summary;

SELECT project_code, metric_name, metric_value
FROM project_audit_log
ORDER BY project_code, metric_name;

COMMIT;
