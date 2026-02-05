# Sandbox Backends

Moltis runs LLM-generated commands inside containers to protect your host
system. The sandbox backend controls which container technology is used.

## Backend Selection

Configure in `moltis.toml`:

```toml
[tools.exec.sandbox]
backend = "auto"          # default — picks the best available
# backend = "docker"      # force Docker
# backend = "apple-container"  # force Apple Container (macOS only)
```

With `"auto"` (the default), Moltis picks the strongest available backend:

| Priority | Backend           | Platform | Isolation          |
|----------|-------------------|----------|--------------------|
| 1        | Apple Container   | macOS    | VM (Virtualization.framework) |
| 2        | Docker            | any      | Linux namespaces / cgroups    |
| 3        | none (host)       | any      | no isolation                  |

## Apple Container (recommended on macOS)

[Apple Container](https://github.com/apple/container) runs each sandbox in a
lightweight virtual machine using Apple's Virtualization.framework. Every
container gets its own kernel, so a kernel exploit inside the sandbox cannot
reach the host — unlike Docker, which shares the host kernel.

### Install

Download the signed installer from GitHub:

```bash
# Download the installer package
gh release download --repo apple/container --pattern "container-installer-signed.pkg" --dir /tmp

# Install (requires admin)
sudo installer -pkg /tmp/container-installer-signed.pkg -target /

# First-time setup — downloads a default Linux kernel
container system start
```

Alternatively, build from source with `brew install container` (requires
Xcode 26+).

### Verify

```bash
container --version
# Run a quick test
container run --rm ubuntu echo "hello from VM"
```

Once installed, restart `moltis gateway` — the startup banner will show
`sandbox: apple-container backend`.

## Docker

Docker is supported on macOS, Linux, and Windows. On macOS it runs inside a
Linux VM managed by Docker Desktop, so it is reasonably isolated but adds more
overhead than Apple Container.

Install from https://docs.docker.com/get-docker/

## No sandbox

If neither runtime is found, commands execute directly on the host. The
startup banner will show a warning. This is **not recommended** for untrusted
workloads.

## Per-session overrides

The web UI allows toggling sandboxing per session and selecting a custom
container image. These overrides persist across gateway restarts.

## Resource limits

```toml
[tools.exec.sandbox.resource_limits]
memory_limit = "512M"
cpu_quota = 1.0
pids_max = 256
```

## Network Policy

Control network access for sandboxed containers with the `network` setting:

```toml
[tools.exec.sandbox]
network = "blocked"   # default — no network access
# network = "trusted" # filtered access via domain allowlist
# network = "open"    # unrestricted network access
```

### Blocked (default)

Containers run with `--network=none`. No outbound connections are possible.
This is the safest option for untrusted workloads.

### Trusted

Containers join an isolated Docker network where the only external gateway is
a filtering HTTP CONNECT proxy. Outbound connections are checked against a
domain allowlist:

```toml
[tools.exec.sandbox]
network = "trusted"
trusted_domains = [
    "github.com",
    "*.github.com",
    "api.openai.com",
    "registry.npmjs.org",
]
```

**Domain patterns:**

| Pattern | Matches |
|---------|---------|
| `github.com` | Exactly `github.com` |
| `*.github.com` | Any subdomain: `api.github.com`, `raw.github.com`, plus `github.com` itself |
| `*` | Everything (effectively disables filtering) |

**Interactive approval:** When a container tries to connect to a domain not in
the allowlist, the gateway broadcasts a `network.domain.requested` event. The
web UI shows a toast notification where operators can approve or deny the
request. Approved domains are remembered for the session.

**How it works:**

1. Containers are launched on an isolated Docker network (`moltis-trusted-net`)
2. Environment variables `HTTP_PROXY` and `HTTPS_PROXY` point to the gateway's
   built-in proxy (port 18791)
3. The proxy intercepts all HTTP/HTTPS traffic and checks domains against the
   allowlist before forwarding

### Open

Containers use the default Docker bridge network with unrestricted internet
access. Use this only for trusted workloads or when network access is required
and domain filtering is impractical.

```toml
[tools.exec.sandbox]
network = "open"
```

### Backward compatibility

The legacy `no_network` boolean is still accepted:

```toml
no_network = true   # equivalent to network = "blocked"
no_network = false  # equivalent to network = "open"
```

## Metrics

When the `metrics` feature is enabled, the sandbox and network proxy expose:

| Metric | Type | Description |
|--------|------|-------------|
| `domain_checks_total` | counter | Domain filter checks by result and source |
| `domain_approval_requests_total` | counter | Interactive approval requests |
| `domain_approval_decisions_total` | counter | Approval outcomes |
| `domain_approval_wait_duration_seconds` | histogram | Time waiting for operator decision |
| `proxy_connections_total` | counter | Total proxy connections |
| `proxy_connections_active` | gauge | Currently active proxy connections |
| `proxy_requests_total` | counter | Requests by method and result |
| `proxy_bytes_transferred_total` | counter | Bytes proxied by direction |
| `proxy_tunnel_duration_seconds` | histogram | HTTPS tunnel lifetime |
