# Google Gemini Provider

Moltis supports Google Gemini models through two authentication methods:

1. **API Key** (`gemini`) - Direct API key authentication
2. **OAuth** (`gemini-oauth`) - Browser-based authentication with your Google account

## API Key Provider

The simplest way to use Gemini. Get an API key from [Google AI Studio](https://aistudio.google.com/apikey) and set it:

```bash
export GEMINI_API_KEY=your_api_key_here
```

Or add it to your `moltis.toml`:

```toml
[providers.gemini]
api_key = "your_api_key_here"
```

## OAuth Provider

The OAuth provider allows users to authenticate with their Google account. **API usage is billed to the user's Google account**, not to the application developer. This is the recommended approach for distributed applications.

### How It Works

1. User initiates login in the Moltis UI
2. Browser opens to Google OAuth consent screen
3. User authenticates with their Google account
4. Browser redirects to local callback server (port 1456)
5. Moltis exchanges the authorization code for tokens using PKCE
6. Tokens are stored securely for future use

### Technical Details

- **Flow**: Authorization Code with PKCE (no client secret required)
- **Scopes**: `generative-language.retriever`, `cloud-platform`
- **Token refresh**: Automatic with 5-minute buffer before expiry
- **Storage**: Tokens stored in `~/.moltis/oauth_tokens.json`

### For Application Developers

To enable Gemini OAuth in your Moltis deployment, you need to create a Google Cloud OAuth client and update the client ID in the codebase.

#### Step 1: Create a Google Cloud Project

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select an existing one
3. Enable the **Generative Language API**:
   - Go to APIs & Services > Library
   - Search for "Generative Language API"
   - Click Enable

#### Step 2: Configure OAuth Consent Screen

1. Go to APIs & Services > OAuth consent screen
2. Select **External** user type (or Internal if using Google Workspace)
3. Fill in the required fields:
   - App name: `Moltis` (or your app name)
   - User support email: your email
   - Developer contact: your email
4. Add scopes:
   - `https://www.googleapis.com/auth/generative-language.retriever`
   - `https://www.googleapis.com/auth/cloud-platform`
5. Add test users if in testing mode

#### Step 3: Create OAuth Credentials

1. Go to APIs & Services > Credentials
2. Click **Create Credentials** > **OAuth client ID**
3. Select **Desktop app** as the application type
4. Name it (e.g., "Moltis Desktop")
5. Click Create
6. Copy the **Client ID** (you don't need the client secret for PKCE)

#### Step 4: Update the Client ID

Replace the placeholder in `crates/oauth/src/defaults.rs`:

```rust
m.insert("gemini-oauth".into(), OAuthConfig {
    client_id: "YOUR_CLIENT_ID_HERE.apps.googleusercontent.com".into(),
    // ... rest of config
});
```

### Security Notes

- The client ID is **not a secret** - it's safe to embed in distributed applications
- PKCE (Proof Key for Code Exchange) prevents authorization code interception attacks
- No client secret is needed because PKCE provides equivalent security
- Tokens are stored locally on the user's machine
- API usage and billing is tied to the user's Google account

## Supported Models

Both providers support the same models:

| Model ID | Description |
|----------|-------------|
| `gemini-2.5-pro-preview-06-05` | Gemini 2.5 Pro (latest) |
| `gemini-2.5-flash-preview-05-20` | Gemini 2.5 Flash (latest) |
| `gemini-2.0-flash` | Gemini 2.0 Flash |
| `gemini-2.0-flash-lite` | Gemini 2.0 Flash Lite |
| `gemini-1.5-pro` | Gemini 1.5 Pro |
| `gemini-1.5-flash` | Gemini 1.5 Flash |

All models support:
- 1M token context window
- Tool/function calling
- Streaming responses
- System instructions

## Configuration

### Selecting a specific model

```toml
[providers.gemini]
model = "gemini-2.5-pro-preview-06-05"

[providers.gemini-oauth]
model = "gemini-2.0-flash"
```

### Disabling a provider

```toml
[providers.gemini]
enabled = false
```

## Troubleshooting

### "not logged in to gemini-oauth"

The OAuth flow hasn't been completed. Click the login button in the Moltis UI to authenticate.

### "token expired and no refresh token available"

The stored tokens are invalid. Clear them and re-authenticate:
- Delete `~/.moltis/oauth_tokens.json` or the gemini-oauth entry within it
- Re-authenticate through the UI

### OAuth callback timeout

If the browser doesn't redirect properly:
1. Check that port 1456 is not blocked by a firewall
2. Ensure no other application is using port 1456
3. Try the authentication flow again

### API quota errors

Gemini has usage quotas. Check your quota in the [Google Cloud Console](https://console.cloud.google.com/apis/api/generativelanguage.googleapis.com/quotas).
