-- 这个文件是幂等的, 新建库和升级已有库都能直接跑: 全库跑一遍, 已经存在的东西会跳过。
--
-- 注意 CREATE TABLE IF NOT EXISTS 只按表名跳过, 不会比对列定义。所以对已有库做过
-- 结构调整的地方, 都在对应表下面补了等价的 ALTER ... IF NOT EXISTS, 两类库跑完
-- 的最终结构是一致的。

-- 1. 站点配置表 (Site)
CREATE TABLE IF NOT EXISTS "site" (
    "id" VARCHAR(36) PRIMARY KEY,
    "site_name" VARCHAR(255),
    "site_icon" VARCHAR(500),
    "banner_image" VARCHAR(500),
    "created_at" BIGINT NOT NULL,
    "site_secret" VARCHAR(255),
    "about_content" TEXT,
    "icp" VARCHAR(100),
    "copyright" VARCHAR(255)
);

COMMENT ON COLUMN "site"."id" IS '站点ID';
COMMENT ON COLUMN "site"."site_name" IS '网站名称';
COMMENT ON COLUMN "site"."site_icon" IS '网站图标URL';
COMMENT ON COLUMN "site"."banner_image" IS 'Banner图URL';
COMMENT ON COLUMN "site"."created_at" IS '创建时间';
COMMENT ON COLUMN "site"."site_secret" IS '管理员密码散列 (argon2 PHC 串)。禁止被任何读接口返回。为空表示尚未设置密码';
COMMENT ON COLUMN "site"."about_content" IS '关于内容';
COMMENT ON COLUMN "site"."icp" IS 'ICP备案号';
COMMENT ON COLUMN "site"."copyright" IS '版权信息';

-- 2. 文章表 (Article)
CREATE TABLE IF NOT EXISTS "article" (
    "id" VARCHAR(36) PRIMARY KEY,
    "title" VARCHAR(255) NOT NULL,
    "slug" VARCHAR(255) NOT NULL,
    "synopsis" TEXT NOT NULL,
    "cover" VARCHAR(500),
    "content" TEXT NOT NULL,
    "likes" BIGINT NOT NULL,
    "status" VARCHAR(50) NOT NULL,
    "created_at" BIGINT NOT NULL,
    "updated_at" BIGINT,
    "deleted_at" BIGINT
);

COMMENT ON COLUMN "article"."id" IS '文章ID';
COMMENT ON COLUMN "article"."title" IS '标题';
COMMENT ON COLUMN "article"."slug" IS 'URL别名';
COMMENT ON COLUMN "article"."synopsis" IS '摘要';
COMMENT ON COLUMN "article"."cover" IS '封面图URL';
COMMENT ON COLUMN "article"."content" IS '文章内容';
COMMENT ON COLUMN "article"."likes" IS '点赞数';
COMMENT ON COLUMN "article"."status" IS '状态';
COMMENT ON COLUMN "article"."created_at" IS '创建时间';
COMMENT ON COLUMN "article"."updated_at" IS '更新时间';
COMMENT ON COLUMN "article"."deleted_at" IS '删除时间';

-- 3. 文章浏览量记录表 (ArticleView)
CREATE TABLE IF NOT EXISTS "article_view" (
    "id" VARCHAR(36) PRIMARY KEY,
    "article_id" VARCHAR(36) NOT NULL,
    "viewed_at" BIGINT NOT NULL
);

COMMENT ON COLUMN "article_view"."id" IS '浏览量记录ID';
COMMENT ON COLUMN "article_view"."article_id" IS '关联的文章ID (无外键约束, 由应用层保证)';
COMMENT ON COLUMN "article_view"."viewed_at" IS '查看时间';

-- 4. 文章标签表 (ArticleTag)
CREATE TABLE IF NOT EXISTS "article_tag" (
    "id" VARCHAR(36) PRIMARY KEY,
    "article_id" VARCHAR(36) NOT NULL,
    "tag_content" VARCHAR(100) NOT NULL
);

COMMENT ON COLUMN "article_tag"."id" IS '标签记录ID';
COMMENT ON COLUMN "article_tag"."article_id" IS '关联的文章ID (无外键约束, 由应用层保证)';
COMMENT ON COLUMN "article_tag"."tag_content" IS '标签内容';

-- 5. 友情链接表 (FriendLink)
CREATE TABLE IF NOT EXISTS "friend_link" (
    "id" VARCHAR(36) PRIMARY KEY,
    "site_name" VARCHAR(255) NOT NULL,
    "site_url" VARCHAR(500) NOT NULL,
    "site_intro" VARCHAR(500) NOT NULL,
    "site_icon" VARCHAR(500),
    "email" VARCHAR(500),
    "feedback_status" VARCHAR(50) NOT NULL,
    "created_at" BIGINT NOT NULL
);

COMMENT ON COLUMN "friend_link"."id" IS '友情链接ID';
COMMENT ON COLUMN "friend_link"."site_name" IS '网站名称';
COMMENT ON COLUMN "friend_link"."site_url" IS '网站地址';
COMMENT ON COLUMN "friend_link"."site_intro" IS '网站介绍';
COMMENT ON COLUMN "friend_link"."site_icon" IS '网站图标URL';
COMMENT ON COLUMN "friend_link"."feedback_status" IS '友链状态';
COMMENT ON COLUMN "friend_link"."email" IS '通知邮箱';
COMMENT ON COLUMN "friend_link"."created_at" IS '创建时间';

-- 6. 社交链接表 (Social)
CREATE TABLE IF NOT EXISTS "social" (
    "id" VARCHAR(36) PRIMARY KEY,
    "type" VARCHAR(100) NOT NULL,
    "icon" VARCHAR(100),
    "value" VARCHAR(500) NOT NULL,
    "created_at" BIGINT NOT NULL
);

COMMENT ON COLUMN "social"."id" IS '社交链接ID';
COMMENT ON COLUMN "social"."type" IS '类型。只允许 Account / Link 两个字面量, 不做大小写转换';
COMMENT ON COLUMN "social"."icon" IS '图标';
COMMENT ON COLUMN "social"."value" IS '值';
COMMENT ON COLUMN "social"."created_at" IS '创建时间。管理端列表靠它稳定排序';

-- 已有库升级: social 原来没有 created_at。先带默认值补列 (表里已有行也能加),
-- 再把默认值去掉, 让最终结构和全新安装一致 (值一律由应用层显式写入)。
ALTER TABLE "social" ADD COLUMN IF NOT EXISTS "created_at" BIGINT NOT NULL DEFAULT 0;
ALTER TABLE "social" ALTER COLUMN "created_at" DROP DEFAULT;

-- 7. 邮件配置表 (MailConfig)
-- 优先使用 enable = TRUE 的那条; 没有启用记录时回退到环境变量。
CREATE TABLE IF NOT EXISTS "mail_config" (
    "id" VARCHAR(36) PRIMARY KEY,
    "host" VARCHAR(255) NOT NULL,
    "port" INTEGER NOT NULL,
    "username" VARCHAR(255) NOT NULL,
    "password" VARCHAR(500) NOT NULL,
    "from_addr" VARCHAR(255) NOT NULL,
    "enable" BOOLEAN NOT NULL DEFAULT FALSE,
    "created_at" BIGINT NOT NULL
);

COMMENT ON COLUMN "mail_config"."id" IS '邮件配置ID';
COMMENT ON COLUMN "mail_config"."host" IS 'SMTP 主机';
COMMENT ON COLUMN "mail_config"."port" IS 'SMTP 端口。465 走隐式 TLS, 其余走 STARTTLS';
COMMENT ON COLUMN "mail_config"."username" IS 'SMTP 用户名';
COMMENT ON COLUMN "mail_config"."password" IS 'SMTP 密码。需要原样交给 SMTP 服务器所以必须可逆存储, 但禁止被任何读接口返回';
COMMENT ON COLUMN "mail_config"."from_addr" IS '发件人地址';
COMMENT ON COLUMN "mail_config"."enable" IS '是否启用。同时只有一条应当为 TRUE';
COMMENT ON COLUMN "mail_config"."created_at" IS '创建时间';

-- ---------------------------------------------------------------------------
-- 索引
--
-- 每一条都是照着 api/repo/ 里真实存在的查询建的, 注释里写了它服务哪个查询。
-- 注意: 部分索引 (WHERE ...) 只有在查询条件能**蕴含**索引谓词时才会被用上,
-- 所以改查询条件时这些索引可能会悄悄失效 —— 用 EXPLAIN 验证。
-- ---------------------------------------------------------------------------

-- --- article ---

-- 公开列表 + 公开计数 (最热路径):
--   WHERE deleted_at IS NULL AND status = 'Pub' ORDER BY created_at DESC LIMIT/OFFSET
--   SELECT COUNT(*) ... 同样的条件
-- 有序扫描, 不需要额外排序; 计数可以走 index-only scan。
CREATE INDEX IF NOT EXISTS "idx_article_pub_created_at"
    ON "article" ("created_at" DESC)
    WHERE "deleted_at" IS NULL AND "status" = 'Pub';

-- 管理端活跃列表:
--   WHERE deleted_at IS NULL [AND status = ?] ORDER BY created_at DESC LIMIT/OFFSET
-- 上面那个部分索引用不了 (它的谓词更强, 反向不蕴含), 所以单建一个。
CREATE INDEX IF NOT EXISTS "idx_article_live_created_at"
    ON "article" ("created_at" DESC)
    WHERE "deleted_at" IS NULL;

-- 管理端回收站: WHERE deleted_at IS NOT NULL ORDER BY created_at DESC
CREATE INDEX IF NOT EXISTS "idx_article_trash_created_at"
    ON "article" ("created_at" DESC)
    WHERE "deleted_at" IS NOT NULL;

-- 回收站里的文章不参与唯一性判断, 所以 slug 用部分唯一索引, 允许同一 slug 被复用。
CREATE UNIQUE INDEX IF NOT EXISTS "uniq_article_slug_live"
    ON "article" ("slug")
    WHERE "deleted_at" IS NULL;

-- --- article_tag ---

-- 列表投影里的 array_agg(...) GROUP BY article_id、详情里的 WHERE article_id = $1、
-- 以及硬删除时的 DELETE ... WHERE article_id = $1。
-- 带上 tag_content 是为了让聚合内部的 ORDER BY tag_content 直接由索引顺序满足,
-- 不必给每个分组再做一次排序。
CREATE INDEX IF NOT EXISTS "idx_article_tag_article_id_tag"
    ON "article_tag" ("article_id", "tag_content");

-- --- article_view ---

-- 列表投影里的 COUNT(*) GROUP BY article_id (每次列表查询都会做一次全表聚合)、
-- 详情里的 WHERE article_id = $1、以及管理端的
-- WHERE article_id = $1 ORDER BY viewed_at DESC (有序, 不用排序)。
-- 前导列是 article_id, 所以整表聚合可以走 index-only scan, 不回表。
CREATE INDEX IF NOT EXISTS "idx_article_view_article_id_viewed_at"
    ON "article_view" ("article_id", "viewed_at" DESC);

-- 定时清理: DELETE FROM article_view WHERE viewed_at < $1。
-- 必须单独建: 上面那个索引的前导列是 article_id, 对按时间的范围删除没用。
-- 这张表是只增的, 又每天批量删一次, 建议确认 autovacuum 跟得上:
--   ALTER TABLE article_view SET (autovacuum_vacuum_scale_factor = 0.05);
CREATE INDEX IF NOT EXISTS "idx_article_view_viewed_at"
    ON "article_view" ("viewed_at");

-- 已有库升级: 这个索引已被上面的 (article_id, viewed_at DESC) 完全覆盖
-- (它就是个更弱的前缀), 留着只会白白拖慢写入。
DROP INDEX IF EXISTS "idx_article_view_article_id";

-- --- friend_link ---

-- 公开列表 + 公开计数:
--   WHERE feedback_status = 'Pass' ORDER BY created_at DESC LIMIT/OFFSET
CREATE INDEX IF NOT EXISTS "idx_friend_link_pass_created_at"
    ON "friend_link" ("created_at" DESC)
    WHERE "feedback_status" = 'Pass';

CREATE UNIQUE INDEX IF NOT EXISTS "uniq_friend_link_site_url"
    ON "friend_link" ("site_url");

-- --- 刻意不建索引的表 ---
-- site / social / mail_config 都是个位数的行数 (站点单行、社交链接几条、
-- 邮件配置一条), 顺序扫描比走索引还快, 加索引只会增加写放大和规划开销。
-- 这几张表的查询见 api/repo/site.rs、social.rs、mail.rs。