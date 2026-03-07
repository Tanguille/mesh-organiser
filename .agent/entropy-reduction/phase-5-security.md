# Phase 5: Security Review
**Completed:** 2026-03-07

## Findings

### 1. SQL Injection Vulnerabilities (CRITICAL)
Multiple locations in the database layer use string interpolation with user-provided data in SQL queries:

- **db/src/model_db.rs:73,80,84,88,113,116** - User ID and filter arrays interpolated directly into query
- **db/src/group_db.rs:242,244-248,450,455** - Model/group IDs joined and interpolated
- **db/src/label_db.rs:255-260,283-287** - Label and model IDs interpolated into DELETE queries
- **db/src/blob_db.rs:35,134** - Blob IDs interpolated

While the application uses `sqlx::query!` macro for most queries (which provides compile-time checking), the dynamic IN-clause construction uses `format!()` which bypasses parameterization. The `join()` function converts i64 values to strings, but if any user-controlled string values reach these paths, injection is possible.

### 2. Missing Rate Limiting (HIGH)
No rate limiting is implemented on:
- Authentication endpoints (`/login/password`, `/login/token`)
- Share access endpoints
- Model download endpoints
- File upload endpoints

This allows brute-force attacks on credentials and DoS via resource exhaustion.

### 3. Insecure CORS Configuration (HIGH)
**src-tauri/src/web_server.rs:21-24**
```rust
let cors = Cors::default()
    .allow_any_origin()
    .allow_any_method()
    .allow_any_header();
```

The embedded web server allows any origin, which could allow malicious websites to make requests to the local server if they can determine the port.

### 4. Weak Session Security (MEDIUM)
**web/src/app.rs:209-212**
```rust
let session_layer = SessionManagerLayer::new(session_store)
    .with_secure(false)  // Cookie secure flag disabled
    .with_expiry(Expiry::OnInactivity(Duration::days(7)))
    .with_signed(key);
```

Sessions use `with_secure(false)` which allows cookies to be sent over HTTP. In production deployments without TLS, this exposes session tokens to network sniffing.

### 5. Local Account Password Generation (MEDIUM)
**web/src/app.rs:129-143**
When `LOCAL_ACCOUNT_PASSWORD` is not set, the application generates a password from a random key. This generated password is stored in the database but never displayed to the user, potentially leaving accounts inaccessible. The auto-generated password should be logged or displayed on first startup.

### 6. Missing Input Validation on File Uploads (MEDIUM)
**web/src/controller/model_controller.rs:313-423**
- No file size limit enforcement (DefaultBodyLimit is disabled at line 240)
- Extension check is case-sensitive and only checks extension, not file content
- No MIME type validation
- No virus/malware scanning

### 7. Command Injection Risk in Slicer Service (LOW-MEDIUM)
**service/src/slicer_service/mod.rs:20-30,60-77**
```rust
pub fn open_with_paths(program: &str, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
    Command::new(program).args(paths).spawn()?;
}
```

The custom slicer path comes from user configuration without validation. While paths are validated to exist, the `parse_command_string` function could be exploited if malicious input reaches the configuration.

### 8. Hardcoded Temporary Directory Patterns (LOW)
Multiple locations use predictable temp directory patterns:
- `meshorganiser_import_action_{}_{}`
- `meshorganiser_download_action_{}`

While nanosecond timestamps provide some entropy, the patterns are predictable. This is mitigated by the use of random_hex_32() in some paths.

### 9. Missing Security Headers (LOW)
No security headers (CSP, HSTS, X-Frame-Options, etc.) are set on HTTP responses from the web server.

### 10. Debug Features in Release (LOW)
**src-tauri/Cargo.toml:19**
```toml
tauri = { workspace = true, features = ["protocol-asset", "devtools"] }
```

The `devtools` feature is enabled, which may expose debugging interfaces in production builds.

### 11. Information Disclosure (LOW)
**web/src/app.rs:120-121**
```rust
#[cfg(debug_assertions)]
println!("Generated SQL Query: {}", query.sql());
```

SQL queries are logged in debug mode, which could leak sensitive information if debug builds are deployed.

### 12. URL Download SSRF Potential (LOW)
**service/src/download_file_service.rs:34-98**
The `download_file` function accepts arbitrary URLs without validation. While the file is saved to a temp directory, this could be exploited for:
- Server-Side Request Forgery to internal resources
- Download of malicious files

### 13. Zip Slip Vulnerability (LOW)
**web/src/controller/model_controller.rs:331-371**
When extracting uploaded ZIP files, the code does not validate that extracted file paths stay within the intended extraction directory. This could allow a zip slip attack if malicious ZIP files are uploaded.

### 14. Weak Share ID Generation (LOW)
Share IDs appear to use random_hex_32() which provides 128 bits of entropy - acceptable but should be documented.

## Vulnerabilities Found

| Severity | Category | Location | Description | Remediation |
|----------|----------|----------|-------------|-------------|
| Critical | SQL Injection | db/src/model_db.rs:73,80,84,88 | User input interpolated into SQL via format! | Use QueryBuilder with push_bind for all dynamic values |
| Critical | SQL Injection | db/src/group_db.rs:244-248,455 | Model IDs joined and interpolated | Use parameterized queries with IN clause binding |
| Critical | SQL Injection | db/src/label_db.rs:258-263,285-290 | Label/model IDs interpolated in DELETE | Implement proper parameterization |
| High | Security Misconfiguration | src-tauri/src/web_server.rs:21-24 | CORS allows any origin/method/header | Restrict CORS to specific origins or disable for local-only |
| High | Broken Access Control | web/src/app.rs | No rate limiting on auth endpoints | Implement rate limiting middleware (e.g., tower-governor) |
| Medium | Cryptographic Failure | web/src/app.rs:210 | Session cookies not marked secure | Use with_secure(true) and require HTTPS in production |
| Medium | Insecure Design | web/src/controller/model_controller.rs:240 | No file upload size limits | Set DefaultBodyLimit to reasonable max (e.g., 100MB) |
| Medium | Injection | service/src/slicer_service/mod.rs | Custom slicer path not validated | Validate and sanitize custom slicer paths |
| Low | Security Misconfiguration | src-tauri/Cargo.toml:19 | Devtools enabled in all builds | Disable devtools feature for release builds |
| Low | Information Disclosure | Multiple files | Debug SQL logging enabled | Remove debug logging in production |
| Low | SSRF | service/src/download_file_service.rs:34 | No URL validation on downloads | Validate URLs against allowlist/block internal IPs |
| Low | Path Traversal | web/src/controller/model_controller.rs | Zip extraction without validation | Validate extracted paths stay within target dir |

## Action Items

- [ ] **CRITICAL:** Audit all SQL query construction in db/src/ and replace format!-based IN clauses with proper parameterization using QueryBuilder::push_bind
- [ ] **CRITICAL:** Implement rate limiting on authentication endpoints using tower-governor or similar
- [ ] **HIGH:** Restrict CORS configuration to specific origins or document why permissive CORS is acceptable
- [ ] **HIGH:** Set session cookie secure flag based on environment (true for production)
- [ ] **MEDIUM:** Implement file upload size limits and MIME type validation
- [ ] **MEDIUM:** Add input validation for custom slicer configuration
- [ ] **MEDIUM:** Display or log auto-generated local account password on first startup
- [ ] **LOW:** Disable Tauri devtools feature in release builds
- [ ] **LOW:** Add security headers (CSP, HSTS, X-Frame-Options) to web responses
- [ ] **LOW:** Validate downloaded URLs don't target internal/private IP ranges
- [ ] **LOW:** Implement zip slip protection in file extraction

## Summary Stats
- Total issues: 14
- Critical: 3 | High: 2 | Medium: 4 | Low: 5

## Notes
- The application generally uses sqlx::query! macro correctly for most queries, providing compile-time SQL verification
- Password hashing uses password-auth crate with appropriate algorithms
- HTML escaping is used appropriately in page_controller.rs
- The Tauri desktop app has reduced attack surface compared to the web deployment
- Web deployment should be behind a reverse proxy (nginx/traefik) for additional security controls
