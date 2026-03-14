# Rule: Backend Security & OWASP Standards

**Priority**: P0 — Security vulnerabilities are release blockers.

## OWASP-aligned baseline

1. **Access control**
   - Protect all endpoints with a default-deny posture.
   - Use RBAC or ABAC and enforce resource-level authorization.

2. **Cryptography and secrets**
   - Use Argon2id or bcrypt for password hashing.
   - Store secrets in a dedicated secret manager (Vault/AWS Secrets Manager/etc.).

3. **Injection prevention**
   - Never build SQL via string concatenation; use parameterized queries only.
   - Validate input at system boundaries with typed DTO/schema validation.

4. **Authentication hardening**
   - Implement strong token/session validation, rotation, and auditability.
