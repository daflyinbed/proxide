CREATE TABLE dists (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    name       VARCHAR(512)  NOT NULL,
    path       VARCHAR(1024) NOT NULL,
    size       BIGINT        NOT NULL DEFAULT 0,
    shasum     VARCHAR(128)  DEFAULT NULL,
    integrity  VARCHAR(256)  DEFAULT NULL,
    created_at DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE packages (
    id                 BIGINT AUTO_INCREMENT PRIMARY KEY,
    name               VARCHAR(512) NOT NULL,
    scope              VARCHAR(256) DEFAULT NULL,
    description        TEXT         DEFAULT NULL,
    source             VARCHAR(64)  DEFAULT NULL,
    abbreviated_dist_id BIGINT      DEFAULT NULL,
    full_dist_id       BIGINT       DEFAULT NULL,
    created_at         DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at         DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_name (name),
    KEY idx_scope (scope)
);

CREATE TABLE package_versions (
    id              BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_id      BIGINT        NOT NULL,
    version         VARCHAR(256)  NOT NULL,
    abbrev_dist_id  BIGINT        DEFAULT NULL,
    manifest_dist_id BIGINT       DEFAULT NULL,
    tar_dist_id     BIGINT        DEFAULT NULL,
    readme_dist_id  BIGINT        DEFAULT NULL,
    publish_time    DATETIME      NOT NULL,
    is_pre_release  BOOLEAN       NOT NULL DEFAULT FALSE,
    padding_version VARCHAR(256)  DEFAULT NULL,
    created_at      DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      DATETIME      NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_package_version (package_id, version),
    KEY idx_publish_time (publish_time),
    KEY idx_pkg_id_pv_pre_rel_ver (package_id, padding_version, is_pre_release, version),
    CONSTRAINT fk_pv_package FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE
);

CREATE TABLE package_tags (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_id BIGINT       NOT NULL,
    tag        VARCHAR(256) NOT NULL,
    version    VARCHAR(256) NOT NULL,
    created_at DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_package_tag (package_id, tag),
    CONSTRAINT fk_pt_package FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE
);

CREATE TABLE change_stream_cursors (
    id         BIGINT PRIMARY KEY,
    since      VARCHAR(256) NOT NULL,
    updated_at DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
);

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

CREATE TABLE users (
    id                  BIGINT AUTO_INCREMENT PRIMARY KEY,
    name                VARCHAR(256) NOT NULL,
    email               VARCHAR(256) DEFAULT NULL,
    upstream_name       VARCHAR(64)  NOT NULL DEFAULT '',
    password_salt       VARCHAR(100) DEFAULT NULL,
    password_integrity  VARCHAR(512) DEFAULT NULL,
    created_at          DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_name_upstream (name, upstream_name)
);

CREATE TABLE tokens (
    id              BIGINT AUTO_INCREMENT PRIMARY KEY,
    token_key       VARCHAR(128) NOT NULL UNIQUE,
    name            VARCHAR(256) NOT NULL,
    user_id         BIGINT       NOT NULL,
    is_readonly     BOOLEAN      NOT NULL DEFAULT FALSE,
    allowed_scopes  TEXT         DEFAULT NULL,
    cidr_whitelist  TEXT         DEFAULT NULL,
    expired_at      DATETIME     DEFAULT NULL,
    created_at      DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    last_used_at    DATETIME     DEFAULT NULL,
    KEY idx_user_id (user_id),
    CONSTRAINT fk_token_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE maintainers (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_id BIGINT   NOT NULL,
    user_id    BIGINT   NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_package_user (package_id, user_id),
    CONSTRAINT fk_maintainer_package FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    CONSTRAINT fk_maintainer_user FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE package_downloads (
    id                 BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_version_id BIGINT           NOT NULL,
    year               SMALLINT UNSIGNED NOT NULL,
    month              TINYINT UNSIGNED  NOT NULL,
    d01  INT UNSIGNED NOT NULL DEFAULT 0,
    d02  INT UNSIGNED NOT NULL DEFAULT 0,
    d03  INT UNSIGNED NOT NULL DEFAULT 0,
    d04  INT UNSIGNED NOT NULL DEFAULT 0,
    d05  INT UNSIGNED NOT NULL DEFAULT 0,
    d06  INT UNSIGNED NOT NULL DEFAULT 0,
    d07  INT UNSIGNED NOT NULL DEFAULT 0,
    d08  INT UNSIGNED NOT NULL DEFAULT 0,
    d09  INT UNSIGNED NOT NULL DEFAULT 0,
    d10  INT UNSIGNED NOT NULL DEFAULT 0,
    d11  INT UNSIGNED NOT NULL DEFAULT 0,
    d12  INT UNSIGNED NOT NULL DEFAULT 0,
    d13  INT UNSIGNED NOT NULL DEFAULT 0,
    d14  INT UNSIGNED NOT NULL DEFAULT 0,
    d15  INT UNSIGNED NOT NULL DEFAULT 0,
    d16  INT UNSIGNED NOT NULL DEFAULT 0,
    d17  INT UNSIGNED NOT NULL DEFAULT 0,
    d18  INT UNSIGNED NOT NULL DEFAULT 0,
    d19  INT UNSIGNED NOT NULL DEFAULT 0,
    d20  INT UNSIGNED NOT NULL DEFAULT 0,
    d21  INT UNSIGNED NOT NULL DEFAULT 0,
    d22  INT UNSIGNED NOT NULL DEFAULT 0,
    d23  INT UNSIGNED NOT NULL DEFAULT 0,
    d24  INT UNSIGNED NOT NULL DEFAULT 0,
    d25  INT UNSIGNED NOT NULL DEFAULT 0,
    d26  INT UNSIGNED NOT NULL DEFAULT 0,
    d27  INT UNSIGNED NOT NULL DEFAULT 0,
    d28  INT UNSIGNED NOT NULL DEFAULT 0,
    d29  INT UNSIGNED NOT NULL DEFAULT 0,
    d30  INT UNSIGNED NOT NULL DEFAULT 0,
    d31  INT UNSIGNED NOT NULL DEFAULT 0,
    UNIQUE KEY uk_pv_ym (package_version_id, year, month),
    KEY idx_pv_y (package_version_id, year),
    CONSTRAINT fk_pd_pv FOREIGN KEY (package_version_id) REFERENCES package_versions(id) ON DELETE CASCADE
);

CREATE TABLE upstream_package_downloads (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_id  BIGINT            NOT NULL,
    year        SMALLINT UNSIGNED NOT NULL,
    month       TINYINT UNSIGNED  NOT NULL,
    d01  INT UNSIGNED NOT NULL DEFAULT 0,
    d02  INT UNSIGNED NOT NULL DEFAULT 0,
    d03  INT UNSIGNED NOT NULL DEFAULT 0,
    d04  INT UNSIGNED NOT NULL DEFAULT 0,
    d05  INT UNSIGNED NOT NULL DEFAULT 0,
    d06  INT UNSIGNED NOT NULL DEFAULT 0,
    d07  INT UNSIGNED NOT NULL DEFAULT 0,
    d08  INT UNSIGNED NOT NULL DEFAULT 0,
    d09  INT UNSIGNED NOT NULL DEFAULT 0,
    d10  INT UNSIGNED NOT NULL DEFAULT 0,
    d11  INT UNSIGNED NOT NULL DEFAULT 0,
    d12  INT UNSIGNED NOT NULL DEFAULT 0,
    d13  INT UNSIGNED NOT NULL DEFAULT 0,
    d14  INT UNSIGNED NOT NULL DEFAULT 0,
    d15  INT UNSIGNED NOT NULL DEFAULT 0,
    d16  INT UNSIGNED NOT NULL DEFAULT 0,
    d17  INT UNSIGNED NOT NULL DEFAULT 0,
    d18  INT UNSIGNED NOT NULL DEFAULT 0,
    d19  INT UNSIGNED NOT NULL DEFAULT 0,
    d20  INT UNSIGNED NOT NULL DEFAULT 0,
    d21  INT UNSIGNED NOT NULL DEFAULT 0,
    d22  INT UNSIGNED NOT NULL DEFAULT 0,
    d23  INT UNSIGNED NOT NULL DEFAULT 0,
    d24  INT UNSIGNED NOT NULL DEFAULT 0,
    d25  INT UNSIGNED NOT NULL DEFAULT 0,
    d26  INT UNSIGNED NOT NULL DEFAULT 0,
    d27  INT UNSIGNED NOT NULL DEFAULT 0,
    d28  INT UNSIGNED NOT NULL DEFAULT 0,
    d29  INT UNSIGNED NOT NULL DEFAULT 0,
    d30  INT UNSIGNED NOT NULL DEFAULT 0,
    d31  INT UNSIGNED NOT NULL DEFAULT 0,
    UNIQUE KEY uk_pkg_ym (package_id, year, month),
    KEY idx_ym (year, month),
    CONSTRAINT fk_upd_package FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE
);

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
