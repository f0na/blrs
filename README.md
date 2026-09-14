# blrs — 博客后端

Rust + axum 0.8, 部署在 Vercel Functions 上。Postgres 存数据, Valkey 只做缓存与计数。

## 接口

### 公开 (BFF)

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/home` | 站点信息 + 社交链接 + 文章列表 (`?page&page_size`) |
| GET | `/about` | 站点信息 (含关于正文) + 社交链接 |
| GET | `/friends` | 站点信息 + 社交链接 + 友链列表 (`?page&page_size`) |
| GET | `/articles/{id}` | 文章详情, 顺带记一次浏览 |
| GET | `/search` | 搜索文章 (`?q` 关键词, `&page&page_size`)。空关键词返回空列表, 不报错 |
| POST | `/friends/apply` | 提交友链申请, 落库为待审核 |
| POST | `/articles/{id}/like` | 点赞 |

### 管理端

除 `POST /admin/login` 外全部需要 `Authorization: Bearer <JWT>`。

| 资源 | 路由 |
|---|---|
| 认证 | `POST /admin/login`, `POST /admin/password`, `GET /admin/me` |
| site | `GET /admin/site`, `POST /admin/site`, `PUT /admin/site` |
| article | `GET /admin/articles` (`?page&page_size&deleted&status`), `GET /admin/articles/{id}`, `POST /admin/articles`, `PUT /admin/articles/{id}`, `DELETE /admin/articles/{id}` (软删), `POST /admin/articles/{id}/restore`, `DELETE /admin/articles/{id}/hard` |
| article 浏览记录 | `GET /admin/articles/{id}/views` |
| friend | `GET /admin/friends` (`?status`), `POST /admin/friends`, `PUT /admin/friends/{id}`, `POST /admin/friends/{id}/approve`, `POST /admin/friends/{id}/refuse`, `DELETE /admin/friends/{id}` |
| social | `GET /admin/socials`, `POST /admin/socials`, `PUT /admin/socials/{id}`, `DELETE /admin/socials/{id}` |
| mail-config | `GET /admin/mail-config`, `POST /admin/mail-config`, `PUT /admin/mail-config` |
| 图片 | `POST /admin/blob/token` |
| 搜索索引 | `POST /admin/search/reindex` (全量重建), `DELETE /admin/search/index` (清空) |

### 定时任务

`GET /cron/cleanup-views` — 删除 90 天前的浏览记录。Vercel Cron 每天 03:00 触发, 用 `CRON_SECRET` 鉴权 (平台会自动带上 `Authorization: Bearer <CRON_SECRET>`)。

## 响应格式

所有接口 (含错误) 都是同一个信封:

```json
{ "code": 0, "msg": "ok", "data": { } }
```

`code` 是数字: `0` 成功 / `40000` 参数错误 / `40100` 未登录 / `40300` 无权限 (含 IP 封禁) / `40400` 不存在 / `40900` 冲突 / `42200` 约束不满足 / `50000` 内部错误 / `50001` 数据库 / `50002` 上游依赖 (HTTP 502)。

**内部细节只进日志, 不进响应体。** 出问题时用响应头里的 `x-request-id` 去日志里捞对应那条。

## 环境变量

已有的数据库/Valkey/Blob 变量见 `.env.example`。另外三个是必须的:

| 变量 | 说明 |
|---|---|
| `JWT_SECRET` | 管理员 JWT 签名密钥。轮换它等于立刻吊销所有登录态。 |
| `CRON_SECRET` | 定时任务鉴权。 |
| `CORS_ORIGINS` | 允许跨域的来源, 逗号分隔。前端与 API 不同域时必填。 |
| `mail_*` (5 个) | 邮件配置。**启动必填**, 缺一个进程就起不来。运行时优先用库里启用的 `mail_config`。 |
| `algolia_*` (3 个) | 搜索配置。**启动必填**。`algolia_search_key` 是 Search-Only Key, 只在搜索接口用; `algolia_write_key` 只在写索引的路径上用。索引名写死在代码里。 |
| `site_*` (4 个) | 站点名称/图标/Banner/版权。**都不是必填**。每次冷启动、迁移之后, 库里还没有填过内容的 `site` 记录就用它们注入一条; 已经有就什么都不做。 |

## 数据库迁移

DDL 的唯一来源是 `migrations/` 目录, SQL 在**编译期**被嵌进二进制, 运行时不需要文件系统。

**迁移在每次冷启动时自动应用**, 没有待应用的迁移时只是一次查询, 不需要你手动跑任何 SQL。
并发冷启动由 Postgres 的 advisory lock 串行化, 不会重复执行。

加一个新迁移:

```bash
# 装了 sqlx-cli 的话 (cargo install sqlx-cli --no-default-features --features postgres)
sqlx migrate add add_something
# 或者直接按 <版本号>_<描述>.sql 的命名手写一个文件
touch migrations/0002_add_something.sql
```

然后往里面写 SQL, 部署即可。

**两条必须遵守的规则:**

1. **不要改已经应用过的迁移文件。** sqlx 会比对校验和, 改了就报 `VersionMismatch` 并且整个进程起不来。要改结构就加新文件。
2. **迁移必须走直连地址。** sqlx 的 Postgres 迁移靠 advisory lock 串行化, 而 Supabase 的 `POSTGRES_URL` 是 PgBouncer 事务池 —— 它不保证同一个会话, 锁会失效。代码里 `db::migration_url()` 优先读 `POSTGRES_URL_NON_POOLING`, 只在你没配的时候才回退到连接池地址并打 WARN。**请确保配了 `POSTGRES_URL_NON_POOLING`。**

**迁移失败会让整个进程起不来**(每个请求 500), 这是刻意的: 结构不确定时继续对外服务只会产生更难排查的问题。修复方式是补一个新的迁移或手工修复库, 而不是回退已应用的文件。

已有的库首次接入时, `0001_init.sql` 里的语句都是 `IF NOT EXISTS` 的, 所以表已经存在也能安全跑过, 之后就会被记为已应用。

## 首次部署

1. 部署即可 —— 迁移会在第一次冷启动时自动建表。确认日志里出现 `数据库结构已是最新` 或 `本次启动了应用了新的数据库迁移`。
2. 调 `POST /admin/login` (此时库里没有密码, **免密通过**), 拿到 token。
3. 立刻用这个 token 调 `POST /admin/password` 设置密码 —— 在设置之前, 任何人都能登录。
4. 站点记录: 配了 `site_*` 环境变量的话冷启动时会自动注入一条, 没配就用 `POST /admin/site` 手动建 —— 两者都没有, 公开接口会返回 500 (站点尚未初始化)。
5. 调一次 `POST /admin/search/reindex`。**索引初始是空的, 不调这一步存量文章一篇都搜不到** ——
   自动索引只在文章的增删改路径上跑, 已经躺在库里的文章没有任何东西会去推它。

## 几个设计取舍

- **登录态是 JWT, 无状态。** 代价是无法主动登出/吊销, 只能靠 3 天 TTL 和轮换 `JWT_SECRET`。
- **没有密码时登录不做任何限制**; 一旦设了密码, 累计错 3 次就把该 IP 封 90 天, 封禁期间该 IP 访问**任何**接口都会被拒。
  失败计数**不会过期**, 只有登录成功才清零 —— 如果给它加个时间窗口, 攻击者只要把节奏放慢到每窗口 2 次就能永远绕过封禁。
  代价是**管理员自己累计输错 3 次也会被锁 90 天**, 而且没有解封接口。解封办法:
  ```bash
  redis-cli -u "$aiven_valkey_service_uri" DEL "blrs:ban:<你的IP>"
  redis-cli -u "$aiven_valkey_service_uri" DEL "blrs:login_fail:<你的IP>"
  ```
  所以设完密码后请把密码存进密码管理器, 别靠记忆。
- **Valkey 是可选依赖。** 启动时连不上只会记 ERROR 并降级运行 (站点上下文每次回源数据库、浏览量不去重、登录封禁失效), 不会导致整个站点起不来。运行期故障同样只记 WARN 后穿透到 Postgres。
- **封禁只认可信来源的 IP。** 日志里用的 `client_ip()` 取的是可伪造的 `x-forwarded-for` 首段 (仅供排查), 而封禁决策走 `trusted_client_ip()`, 只认平台写的头; 拿不到可信来源就放弃封禁判断 —— 宁可漏封, 也不能让人用一个伪造的头把任意 IP 封掉 90 天。**部署后请按下面"验证"的第 1 条实测一次。**
- **图片不由后端中转。** 后端只签发一把限时 5 分钟、限 25MB、限常见图片类型的直传钥匙, 浏览器拿它直接传 Vercel Blob。
- **`friend_link.email` 禁止被任何读接口返回。** 管理端列表用的结构体里干脆没有这个字段; 全代码库只有 `repo::friend::get_for_update` 会读它, 用于审核/拒绝时发通知邮件。
- **`mail_config.password` 同理**, 读接口只返回 `has_password`。`site.site_secret` 也不在任何 SELECT 列表里。
- **搜索走 Algolia, 但后端是唯一的出口。** 索引的读写都在服务端做, 前端既拿不到 `algolia_write_key`, `algolia_search_key` 也不出后端 —— 搜索接口是 `GET /search`, 不是"后端发一把钥匙让前端直连 Algolia"。
- **`/search` 是唯一一个不套 `{site, social, ...}` 页面信封的公开接口。** 它服务的是全局搜索框的下拉, 不是一张页面: 站点信息调用方本来就有, 每次敲键都带一份纯属浪费, 还得为此多查一次站点缓存和一次库。它返回的就是一个 `Pager`, 和 `/admin/articles` 同一种形状。所以它看起来"和其它 BFF 接口不一致"是故意的, 别顺手给它补上站点信息。
- **搜索的空关键词不是错误。** 搜索框长在每一页上, 被清空是常态; 这里返回 `total: 0` 的空列表, 前端就能无条件发请求。要是返回 40000, 前端得为了"别把空串送出去"多加一个分支, 日志也会被刷满。**只有超长 (100 字) 才是参数错误。**
- **索引里只有"能被搜到的文章"**: 已发布且不在回收站。草稿和回收站的文章不是被标记成不可搜, 而是压根不写进去 —— 这样搜索请求不需要任何 filter, 也就不存在"忘了配 `attributesForFaceting` 于是搜出草稿"这种事。
- **正文不进索引。** `article.content` 列上限 50 万字符 (约 1.5MB), 而 Algolia 单条记录上限 100KB、整个索引的平均记录大小还必须压在 10KB 以内, 原样推上去会被拒 (`Record is too big`)。所以搜得到的是标题、摘要、标签。
- **搜索结果不查库**, 直接返回 Algolia 命中里的字段, 因此**不含 `likes` / `views`** —— 这两个数一直在变, 放进索引就等于每次点赞、每次浏览都得重新索引一遍。要点赞数/浏览量就按 `id` 去调 `/articles/{id}`。
- **索引同步是尽力而为的旁路。** 文章的增 / 改 / 软删 / 恢复 / 真删都会在**事务提交之后**调一次 `service::search::sync`, 它内部任何失败都只记 ERROR 日志, 绝不让文章操作跟着失败 —— 和浏览计数、友链审核发信是同一套思路。代价是索引可能和库漂移, 用 `POST /admin/search/reindex` 修。提交之后才同步是刻意的: 先索引再提交的话, 提交失败就会在索引里留下一篇并不存在的文章。
- **`POST /admin/search/reindex` 是"先清空再全推"**, 这样"库删了但索引里还留着"的漂移能一并清掉。代价是中途失败会留下一个不完整的索引 (再点一次即可), 所以它先查库再清空 —— 查询失败的话索引原样不动, 不至于把搜索打空。
- **状态值不做任何大小写/命名转换。** `Draft`/`Pub`、`Review`/`Pass` 直接由 serde 校验, 非法值一律拒绝。
- **两条友链处理路径的先后顺序是刻意相反的:**
  - `approve` — **先提交再发信**。审核结果必须落库, 邮件发不出去只把 `notified` 标成 `false`, 接口照样 200。
  - `refuse` — **先发信再删记录**。拒绝时记录本身就是要删掉的东西, 如果先删再发信而信又发失败了, 管理员手上就没有任何东西可以重试。所以发信失败会返回 502 并且**记录原样保留**, 改好配置再点一次即可。对方压根没留邮箱时没法通知, 直接删除。
- **时间戳统一是 Unix 秒**, 展示时按 UTC+8 格式化 (改 `util::DISPLAY_TZ_OFFSET_SECS` 换时区)。
- **每条数据库查询都必须带 `.persistent(false)`。** 应用的 `POSTGRES_URL` 是 Supabase 的 PgBouncer **事务池**(6543),而 sqlx 默认发**具名**预处理语句(`sqlx_s_1`...)。具名语句是会话级的,事务池会在事务之间把你换到别的后端,于是两个连接各自建的 `sqlx_s_1` 撞在同一个后端上,报 `prepared statement "sqlx_s_1" already exists`。
  注意 `statement_cache_capacity(0)` **治不了这个** —— 语句名带不带 `sqlx_s_` 前缀只取决于 `persistent`。必须用 `.persistent(false)` 让 sqlx 发匿名语句。
  代价是每条查询都要重新 Parse(没有语句缓存),这个体量可以忽略。`repo::tests::every_query_disables_persistent` 会守住这条规则,新查询漏了测试就红。
- **索引是照着 `src/repo/` 里的真实查询建的**, 每条都注明了它服务哪个查询。这里有个容易踩的点: **部分索引只在查询条件能蕴含索引谓词时才会被用上**, 所以 `repo::article::admin_predicate` 用字面量而不是参数化 `CASE` 拼 SQL —— 参数化的写法会让 Postgres 无法证明蕴含关系, 那几个部分索引就全都白建了。改查询条件时请用 `EXPLAIN` 复核。
- **`site_*` 环境变量是"注入一次", 不是"读的时候兜底"。** 每次冷启动、迁移跑完之后, 库里一行都没有 (或只有首次设密码时建的占位记录) 才用它们注一条, 已经有填过名字的记录就碰都不碰 (包括不覆盖)。读路径 (公开接口和管理端) 只认库里那一行, 所以两边看到的是同一份数据, 不会出现"线上在生效、管理台却看不到也改不掉"的幽灵配置; 代价是**改了环境变量不会生效**, 要改站点信息得走 `/admin/site` (`site_name` 一个都没配又没建记录, 公开接口才 500)。注入失败只记 ERROR 不拦启动: 那说明库本身有问题, 拦下来只会连管理端也一起用不了。
- `site`/`social`/`mail_config` **刻意不建索引**: 都是个位数行数, 顺序扫描比走索引还快。

## 验证清单

1. `cargo check` / `cargo clippy -- -D warnings` / `cargo test` 全绿。
2. 部署后用伪造头实测可信 IP:
   ```bash
   curl -H "x-real-ip: 1.2.3.4" -H "x-forwarded-for: 1.2.3.4" https://<your-app>/home
   ```
   确认日志里的 `client_ip` 是**真实出口 IP 而不是 1.2.3.4**。如果 Vercel 没有覆盖这两个头, `util::trusted_client_ip` 必须改成更保守的取法 (或直接返回 `None` 关闭封禁), 否则伪造头就能封掉任意 IP 90 天。
3. 公开接口的响应体里 `grep -iE "email|password|site_secret"` 应当无命中; `/home` 有 `views` 数量但没有任何单条查看时间。
4. 不带 token 打 `GET /admin/articles` 应得 401。
5. 传 `{"status":"draft"}` 建文章应被拒, `"Draft"` 才成立。
6. 故意配错 SMTP 密码:
   - 调 `POST /admin/friends/{id}/approve` → 200 + `notified: false` + ERROR 日志, 且状态仍然是 `Pass`。
   - 调 `POST /admin/friends/{id}/refuse` → 502, 且**记录还在**; 改回正确配置后重试应当成功删除。
7. `GET /home?page=9223372036854775807` 应当返回 40000 而不是 500。
8. 搜索链路:
   - 建一篇 `Pub` 文章 → 立刻搜得到; 改成 `Draft` → 搜不到了; 移入回收站 → 搜不到; 恢复 → 搜得到; 真删 → 搜不到。
   - `GET /search` 不带 `q`、或 `q` 全是空格 → `total: 0` 的空列表 (不是 40000)。带中文和空格 (`?q=%E4%B8%AD%E6%96%87+rust`) 能搜到东西 —— 说明解码生效了。`q` 超过 100 字 → 40000。
   - 响应体里应当**没有** `site` / `social` —— 它不是页面接口。
   - `GET /search` 的响应体里 `grep -iE "algolia|api[-_]?key"` 应当无命中 (密钥不能出现在响应里), 日志里同样不该出现。
   - 不带 token 打 `POST /admin/search/reindex` 应得 401。
   - `POST /admin/search/reindex` 之后, 把库里某篇已发布文章手工删掉再搜一次 —— 索引里那条应当已经没了 (这就是重建比"只推不删"多出来的那一步)。
   - `DELETE /admin/search/index` 之后搜索返回空列表, 再 `reindex` 应当恢复。
