CREATE TABLE package_version_files (
    id                 BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_version_id BIGINT        NOT NULL,
    dist_id            BIGINT        NOT NULL,
    filepath           VARCHAR(750)  NOT NULL,
    content_type       VARCHAR(255)  NOT NULL DEFAULT 'application/octet-stream',
    created_at         DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_pv_filepath (package_version_id, filepath),
    INDEX idx_pv (package_version_id),
    CONSTRAINT fk_pvf_version FOREIGN KEY (package_version_id)
        REFERENCES package_versions(id) ON DELETE CASCADE,
    CONSTRAINT fk_pvf_dist FOREIGN KEY (dist_id)
        REFERENCES dists(id) ON DELETE CASCADE
);
