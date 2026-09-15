use axum::http::HeaderMap;
use chrono::Utc;

use crate::model::err::{AppError, AppResult};

/// 当前 Unix 秒。
pub fn now_secs() -> i64 {
    Utc::now().timestamp()
}

/// 当前 Unix 毫秒。Blob 签发接口用的是毫秒。
pub fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 客户端 IP, **仅用于日志**。
///
/// `x-forwarded-for` 的第一段完全由客户端控制, 可以随手伪造, 所以这个函数的结果
/// 绝不能用于鉴权、限流或封禁决策 —— 需要做决策时用 [`trusted_client_ip`]。
pub fn client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = header_str(headers, "x-forwarded-for")
        && let Some(first) = xff.split(',').next()
        && !first.trim().is_empty()
    {
        return first.trim().to_string();
    }
    if let Some(v) = header_str(headers, "x-real-ip") {
        return v;
    }
    if let Some(v) = header_str(headers, "x-vercel-forwarded-for") {
        return v;
    }
    "-".to_string()
}

/// 用于**封禁决策**的客户端 IP, 来源必须严格。
///
/// 只认平台写入的头; 取不到可信来源时返回 `None`, 调用方必须据此**放弃封禁判断**
/// (放行)。宁可漏封, 也不能因为一个伪造的头把任意 IP 封掉 90 天。
///
/// 部署到 Vercel 后需要用伪造头实测一次: 见计划里的"验证"第 8 条。如果平台没有
/// 覆盖这些头, 必须把这里改成只认某个可信头, 或者直接返回 `None`。
pub fn trusted_client_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = header_str(headers, "x-real-ip") {
        return Some(v);
    }
    // 退一步取 `x-forwarded-for` 的**最后一段** —— 那一段是最近一跳可信代理追加的,
    // 不是客户端能直接写的位置。绝不取第一段。
    if let Some(xff) = header_str(headers, "x-forwarded-for")
        && let Some(last) = xff.rsplit(',').next()
    {
        let last = last.trim();
        if !last.is_empty() {
            return Some(last.to_string());
        }
    }
    None
}

/// 必填文本字段校验: 去掉首尾空白, 拒绝空串和超长内容。
/// 长度按字符数算, 所以中文不会被当成 3 个字节误判超长。
pub fn required_text(value: &str, field: &str, max_len: usize) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request(format!("{field}不能为空")));
    }
    if trimmed.chars().count() > max_len {
        return Err(AppError::bad_request(format!("{field}不能超过 {max_len} 字")));
    }
    Ok(trimmed.to_string())
}

/// 选填文本字段校验: 空白视同未填。
pub fn optional_text(
    value: Option<&str>,
    field: &str,
    max_len: usize,
) -> AppResult<Option<String>> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => {
            if v.chars().count() > max_len {
                return Err(AppError::bad_request(format!("{field}不能超过 {max_len} 字")));
            }
            Ok(Some(v.to_string()))
        }
    }
}

/// 友链地址只接受 http/https 绝对地址 —— 免得库里躺一堆 `javascript:` 之类的东西。
pub fn validate_site_url(value: &str) -> AppResult<String> {
    let url = required_text(value, "站点地址", 500)?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::bad_request("站点地址必须以 http:// 或 https:// 开头"));
    }
    Ok(url)
}

/// 通知邮箱的粗校验: 只要求有 `@` 且两侧都非空, 不追求完整 RFC 校验。
///
/// 但必须挡住空白和控制字符 —— 这类值能过这里的粗校验却会让后面构造邮件头时失败,
/// 而那个失败的报错里会带上地址。在入口挡掉更干净。
pub fn validate_email(value: Option<&str>) -> AppResult<Option<String>> {
    let Some(email) = optional_text(value, "邮箱", 500)? else {
        return Ok(None);
    };
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::bad_request("邮箱不能包含空白或控制字符"));
    }
    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| AppError::bad_request("邮箱格式不正确"))?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err(AppError::bad_request("邮箱格式不正确"));
    }
    Ok(Some(email))
}

/// 常量时间比较, 用于比对密钥 (登录口令 / cron 密钥)。
/// 长度不同会提前返回, 这只泄漏长度, 不泄漏内容。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 去掉邮件头里不允许出现的字符, 防头注入 (CRLF)。
pub fn sanitize_header_value(value: &str) -> String {
    value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 把用户提交的文件名收敛成一个安全的 pathname 片段。
///
/// 保留 Unicode 字母数字和 `-_.`, 其余 (含 `/` `\` 空格) 一律替换成 `_`。因为路径
/// 分隔符被消灭, 结果不可能跨越目录; 再裁掉首尾的 `.` 和 `_` 以排除隐藏名和 `..`。
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['.', '_']);
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_filename("/绝对路径.png"), "绝对路径.png");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("a.png"), "a.png");
    }

    #[test]
    fn trusted_ip_ignores_spoofable_first_hop() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 10.0.0.1".parse().unwrap());
        // 客户端能写的只有第一段, 所以只能采信最后一段
        assert_eq!(trusted_client_ip(&h).as_deref(), Some("10.0.0.1"));

        let mut h2 = HeaderMap::new();
        h2.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        assert_eq!(trusted_client_ip(&h2).as_deref(), Some("5.6.7.8"));

        assert_eq!(trusted_client_ip(&HeaderMap::new()), None);
    }

    #[test]
    fn sanitize_header_value_removes_crlf() {
        assert_eq!(sanitize_header_value("a\r\nBcc: x@y.z"), "a  Bcc: x@y.z");
    }
}
