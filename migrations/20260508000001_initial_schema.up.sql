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
