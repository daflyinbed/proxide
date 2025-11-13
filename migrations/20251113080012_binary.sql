-- Add migration script here
CREATE TABLE binaries (
  `id` bigint unsigned PRIMARY KEY AUTO_INCREMENT,
  `gmt_created` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) COMMENT 'create time',
  `gmt_modified` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3) COMMENT 'modified time',
  `category` varchar(50) NOT NULL COMMENT 'binary category, e.g.: node, sass',
  `parent` varchar(500) NOT NULL COMMENT 'binary parent name, e.g.: /, /v1.0.0/, /v1.0.0/docs/',
  `name` varchar(200) NOT NULL COMMENT 'binary name, dir should ends with /',
  `is_dir` boolean NOT NULL DEFAULT FALSE COMMENT 'is directory',
  `size` bigint unsigned NOT NULL COMMENT 'binary size in bytes',
  `date` datetime(3) NOT NULL COMMENT 'binary last modified date',
  UNIQUE KEY `uk_category_parent_name` (`parent`, `name`)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb3;

CREATE TABLE tasks (
  `id` bigint unsigned PRIMARY KEY AUTO_INCREMENT,
  `gmt_created` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) COMMENT 'create time',
  `gmt_modified` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3) COMMENT 'modified time',
  `type` varchar(20) NOT NULL COMMENT 'task type',
  `state` varchar(20) NOT NULL COMMENT 'task state',
  `author_id` varchar(24) NOT NULL COMMENT 'create task user id',
  `author_ip` varchar(100) NOT NULL COMMENT 'create task user request ip',
  `data` json DEFAULT NULL COMMENT 'task params',
  `error` longtext COMMENT 'error description'
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb3;

CREATE TABLE `history_tasks` (
  `id` bigint unsigned PRIMARY KEY AUTO_INCREMENT,
  `gmt_created` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) COMMENT 'create time',
  `gmt_modified` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3) COMMENT 'modified time',
  `task_id` varchar(24) NOT NULL COMMENT 'task id',
  `type` varchar(20) NOT NULL COMMENT 'task type',
  `state` varchar(20) NOT NULL COMMENT 'task state',
  `author_id` varchar(24) NOT NULL COMMENT 'create task user id',
  `author_ip` varchar(100) NOT NULL COMMENT 'create task user request ip',
  `data` json NULL COMMENT 'task params',
  `attempts` int unsigned DEFAULT 0 COMMENT 'task execute attempts times',
  `error` longtext COMMENT 'error description'
) ENGINE = InnoDB DEFAULT CHARSET = utf8 COMMENT = 'history task info';

CREATE TABLE `caches` (
  `id` bigint unsigned PRIMARY KEY AUTO_INCREMENT,
  `gmt_created` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) COMMENT 'create time',
  `gmt_modified` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3) COMMENT 'modified time',
  `binary_id` bigint unsigned NOT NULL COMMENT 'binary id',
  `local_path` varchar(1000) NOT NULL COMMENT 'local file path',
  `local_size` bigint unsigned NOT NULL COMMENT 'local file size',
  `access_count` int unsigned NOT NULL DEFAULT 0 COMMENT 'access count'
) ENGINE = InnoDB DEFAULT CHARSET = utf8 COMMENT = 'cache info';
