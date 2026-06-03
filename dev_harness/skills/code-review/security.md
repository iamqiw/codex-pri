# Security Review

## Objective

Identify whether the change introduces security defects, weakens trust boundaries, or omits required validation.

## Review checks

- Check all new inputs for validation, normalization, authorization, and size limits.
- Check whether sensitive values can be logged, serialized, cached, or exposed in error messages.
- Check command execution, file access, network access, and deserialization paths for injection or privilege issues.
- Check whether authentication, authorization, tenancy, or ownership checks remain enforced at the correct boundary.
- Check whether new dependencies, scripts, or generated artifacts introduce supply-chain or execution risk.
- Check whether security-relevant behavior requires tests under `dev_harness/test/versions/`.

## Finding format

- Trust boundary affected.
- Exploit condition or misuse path.
- Impact.
- Required mitigation and test expectation.
