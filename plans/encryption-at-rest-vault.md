# Encryption-at-Rest Vault (XChaCha20-Poly1305)

## Overview

The `moltis-vault` crate encrypts sensitive data at rest using
XChaCha20-Poly1305. A random DEK (Data Encryption Key) is wrapped with a
password-derived KEK via Argon2id. The vault must be password-unlocked once
per process start. Trait-based design allows swapping the encryption backend.

**Scope**: env variables (DB), provider_keys.json, oauth_tokens.json.
**Recovery key**: generated at vault creation, displayed once.
**Activation**: always-on when a password is set.

**Honest limitation**: encryption-at-rest protects cold data (disk theft,
backups, forensics). It does NOT protect against runtime secret leakage via
prompt injection — that requires proxy-based credential injection (see
`plans/proxy-credential-injection.md`).

## Status

**Implemented** (branch `aes-256-gcm`):
- [x] `crates/vault/` crate with full test suite (38 tests)
- [x] XChaCha20-Poly1305 cipher behind `Cipher` trait
- [x] Argon2id KDF, DEK wrapping, recovery key
- [x] Vault state machine (Uninitialized/Sealed/Unsealed)
- [x] Gateway feature flag (`vault`, default-enabled)
- [x] `vault_guard` middleware (423 Locked when sealed)
- [x] Vault API routes (status/unlock/recovery)
- [x] Setup handler: vault init + recovery key
- [x] Login handler: vault unseal + env var migration
- [x] CredentialStore: encrypt/decrypt env vars
- [x] GonData: `vault_status` for frontend

**Remaining**:
- [ ] KeyStore file encryption (provider_keys.json → .enc)
- [ ] TokenStore file encryption (oauth_tokens.json → .enc)
- [ ] Frontend: vault unlock page
- [ ] Frontend: recovery key modal in setup flow
- [ ] Frontend: app.js routing for sealed state

## Key Hierarchy

```
Password ──→ Argon2id(password, salt) ──→ KEK ──→ unwrap(wrapped_dek) ──→ DEK
Recovery ──→ Argon2id(phrase, fixed_salt) ──→ recovery_KEK ──→ unwrap ──→ DEK
```

DEK: random 256-bit, generated once. Held in memory as `Zeroizing<[u8; 32]>`.
Password change re-wraps the DEK — zero data re-encryption.

## Crate Structure

```
crates/vault/
├── Cargo.toml
├── migrations/
│   └── 20260210000000_vault_metadata.sql
└── src/
    ├── lib.rs           # re-exports, run_migrations()
    ├── error.rs         # VaultError (thiserror)
    ├── traits.rs        # Cipher trait for swappable backends
    ├── xchacha20.rs     # XChaCha20-Poly1305 implementation of Cipher
    ├── kdf.rs           # Argon2id key derivation
    ├── key_wrap.rs      # DEK wrapping/unwrapping (uses Cipher trait)
    ├── vault.rs         # Vault struct: state machine, encrypt/decrypt
    ├── recovery.rs      # Recovery key generation and wrapping
    └── migration.rs     # Plaintext-to-encrypted migration helpers
```

## Cipher Trait

```rust
pub trait Cipher: Send + Sync {
    fn version_tag(&self) -> u8;
    fn encrypt(&self, key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, VaultError>;
    fn decrypt(&self, key: &[u8; 32], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, VaultError>;
}
```

Default: `XChaCha20Poly1305Cipher` (version tag `0x01`).

## Encrypted Blob Format

```
[version: 1 byte][nonce: 24 bytes][ciphertext + Poly1305 tag: N + 16 bytes]
```

Base64-encoded for storage in DB or files.

## Database Schema

```sql
CREATE TABLE IF NOT EXISTS vault_metadata (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    version              INTEGER NOT NULL DEFAULT 1,
    kdf_salt             TEXT NOT NULL,
    kdf_params           TEXT NOT NULL,
    wrapped_dek          TEXT NOT NULL,
    recovery_wrapped_dek TEXT,
    recovery_key_hash    TEXT,
    created_at           TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at           TEXT NOT NULL DEFAULT (datetime('now'))
);

-- env_variables gains an encrypted flag:
ALTER TABLE env_variables ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 0;
```

## Vault Public API

```rust
pub struct Vault<C: Cipher = XChaCha20Poly1305Cipher> { ... }

impl<C: Cipher> Vault<C> {
    pub async fn new(pool: SqlitePool) -> Result<Self>;
    pub async fn status(&self) -> Result<VaultStatus>;
    pub async fn initialize(&self, password: &str) -> Result<RecoveryKey>;
    pub async fn unseal(&self, password: &str) -> Result<()>;
    pub async fn unseal_with_recovery(&self, phrase: &str) -> Result<()>;
    pub async fn seal(&self);
    pub async fn change_password(&self, old: &str, new: &str) -> Result<()>;
    pub async fn encrypt_string(&self, plaintext: &str, aad: &str) -> Result<String>;
    pub async fn decrypt_string(&self, b64: &str, aad: &str) -> Result<String>;
    pub async fn is_unsealed(&self) -> bool;
}
```

## Server Startup Flow

```
1. Run migrations: projects → sessions → cron → vault → gateway
2. let vault = Arc::new(Vault::new(pool).await?);
3. match vault.status() {
     Uninitialized → pass vault handle (init happens during setup)
     Sealed → locked mode (vault_guard middleware returns 423)
     Unsealed → shouldn't happen on fresh start
   }
4. CredentialStore::with_vault(pool, config, vault.clone())
5. GatewayState::with_options(..., vault.clone())
6. Continue normal startup
```

## Locked Mode

`vault_guard` middleware in `auth_middleware.rs`:
- Returns `423 Locked` for `/api/*` routes except `/api/auth/*` and `/api/gon`
- Non-API routes pass through (unlock page can load)

## Recovery Key

Format: `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX` (128-bit, alphanumeric).
Derive recovery KEK via Argon2id with fixed salt and lighter params.
Store `recovery_wrapped_dek` + `SHA-256(recovery_key)` in vault_metadata.

## Open Design Questions

1. **KeyStore/TokenStore async refactor**: These are sync (std::Mutex + std::fs).
   The vault encrypt/decrypt are async. Options:
   - Refactor to `tokio::sync::Mutex` + async file I/O
   - Use `tokio::task::spawn_blocking` wrapper
   - Handle encryption at gateway level (current approach for env vars)

2. **Vault reset flow**: If user resets all auth, should vault data be wiped?
   Currently vault metadata persists — user needs to re-initialize after
   setting a new password.
