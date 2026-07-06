CREATE TABLE organizations (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    name        VARCHAR(256) NOT NULL,
    description TEXT         DEFAULT NULL,
    created_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_org_name (name)
);

CREATE TABLE org_members (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    org_id     BIGINT      NOT NULL,
    user_id    BIGINT      NOT NULL,
    role       VARCHAR(16) NOT NULL DEFAULT 'developer',
    created_at DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_org_user (org_id, user_id),
    KEY idx_org_role (org_id, role),
    CONSTRAINT fk_orgmember_org  FOREIGN KEY (org_id)  REFERENCES organizations(id) ON DELETE CASCADE,
    CONSTRAINT fk_orgmember_user FOREIGN KEY (user_id) REFERENCES users(id)          ON DELETE CASCADE
);

CREATE TABLE teams (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    org_id      BIGINT      NOT NULL,
    name        VARCHAR(256) NOT NULL,
    description TEXT         DEFAULT NULL,
    created_at  DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_team_org_name (org_id, name),
    CONSTRAINT fk_team_org FOREIGN KEY (org_id) REFERENCES organizations(id) ON DELETE CASCADE
);

CREATE TABLE team_members (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    team_id    BIGINT   NOT NULL,
    user_id    BIGINT   NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_team_user (team_id, user_id),
    KEY idx_team_user (team_id, user_id),
    CONSTRAINT fk_teammember_team FOREIGN KEY (team_id) REFERENCES teams(id) ON DELETE CASCADE,
    CONSTRAINT fk_teammember_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE TABLE package_team_permissions (
    id         BIGINT AUTO_INCREMENT PRIMARY KEY,
    package_id BIGINT      NOT NULL,
    team_id    BIGINT      NOT NULL,
    permission VARCHAR(16) NOT NULL DEFAULT 'read',
    created_at DATETIME    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uk_pkg_team (package_id, team_id),
    KEY idx_team_perm (team_id, permission),
    CONSTRAINT fk_pkgteam_package FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
    CONSTRAINT fk_pkgteam_team    FOREIGN KEY (team_id)    REFERENCES teams(id)    ON DELETE CASCADE
);
