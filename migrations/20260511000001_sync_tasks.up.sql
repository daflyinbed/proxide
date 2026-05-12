CREATE TABLE sync_tasks (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    name         VARCHAR(512) NOT NULL,
    source       VARCHAR(32)  NOT NULL,
    status       VARCHAR(16)  NOT NULL DEFAULT 'pending',
    attempts     INT          NOT NULL DEFAULT 0,
    max_attempts INT          NOT NULL DEFAULT 3,
    error        TEXT         DEFAULT NULL,
    created_at   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at   DATETIME     DEFAULT NULL,
    finished_at  DATETIME     DEFAULT NULL,
    KEY idx_status_created (status, created_at),
    KEY idx_name_status (name, status)
);
