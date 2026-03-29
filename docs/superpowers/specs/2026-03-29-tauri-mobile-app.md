# Tauri Mobile App for Mesh Organiser - Design Specification

## Overview

This document outlines the design for a Tauri v2 mobile app (Android first, iOS minimal work) that connects to a remote Mesh Organiser server instance. The app shares ~80% of its codebase with the desktop application.

## Key Requirements

1. **Android first** - iOS only if minimal additional work
2. **Maximum code sharing** with desktop app (~80%)
3. **Remote server connectivity** - connects to NAS-based Mesh Organiser instance
4. **No offline capabilities** - relies on running server
5. **Single binary** - same Tauri app for desktop and mobile with platform detection

## Architecture

### Client-Server Model

```
Mobile App ↔ Remote Mesh Organiser Server (HTTP API)
                    ↓
              OrcaSlicer (slicing)
                    ↓
              Printer Communication
```

### Selected Approach: Single Tauri App with Platform Detection

- Same binary for desktop + mobile
- Platform detection in Rust backend (`#[cfg(target_os = "android")]`)
- Platform-specific capabilities in Tauri
- Remote connectivity via `tauri-plugin-http`
- Frontend: same SvelteKit with conditional platform rendering

## Technical Implementation

### Tauri v2 Mobile Support

- **iOS**: WKWebView (Tauri built-in)
- **Android**: Android WebView (Tauri built-in)
- **Min Android SDK**: 24 (Android 7.0)
- **Build command**: `tauri android build`

### Frontend Changes

1. **Platform Detection**
   - Add `PLATFORM` environment variable detection
   - Conditional rendering for mobile UI components
   - Touch-optimized controls

2. **Remote API Integration**
   - Use `tauri-plugin-http` for all API calls
   - Remove local database dependencies
   - Add server URL configuration screen

3. **UI Adaptations**
   - Mobile-friendly navigation (bottom tabs)
   - Larger touch targets
   - Responsive layout adjustments

### Backend Changes (Rust)

1. **Platform Detection**

   ```rust
   #[cfg(target_os = "android")]
   fn platform_specific_setup() { ... }

   #[cfg(target_os = "ios")]
   fn platform_specific_setup() { ... }

   #[cfg(not(any(target_os = "android", target_os = "ios")))]
   fn platform_specific_setup() { ... }
   ```

2. **Disable Desktop Features on Mobile**
   - No local SQLite database
   - No file system access for model storage
   - No thumbnail generation (use server thumbnails)

3. **Add Mobile Commands**
   - `get_platform()` - returns "desktop", "android", "ios"
   - `get_server_url()` - configured server endpoint
   - `set_server_url(url)` - configure remote server

### Capabilities (Tauri)

Platform-specific capabilities in `src-tauri/capabilities/`:

- `desktop.json` - current capabilities
- `mobile.json` - minimal mobile capabilities (http only)
- `android.json` - Android-specific
- `ios.json` - iOS-specific

## Data Flow

### Authentication Flow

1. User opens mobile app
2. App shows server URL configuration screen (first time)
3. User enters server URL (e.g., `https://mesh-organiser.local:9443`)
4. App stores URL and attempts connection
5. Server returns auth challenge (if required)
6. User authenticates via server's auth system (webview redirect)

### Model Browsing Flow

1. App fetches model list from `GET /api/models`
2. Server returns models with thumbnail URLs
3. App displays grid of models with thumbnails
4. User taps model → detail view
5. Detail view shows full model info from `GET /api/models/{id}`

### Slicing Flow

1. User selects model and adjusts settings
2. App sends `POST /api/slicer/slice` with settings
3. Server processes slice via OrcaSlicer
4. Server returns sliced G-code
5. App displays preview (image from server)
6. User sends to printer via `POST /api/printers/{id}/print`

## API Endpoints Used

### Model Operations

- `GET /api/models` - List models
- `GET /api/models/{id}` - Model details
- `GET /api/models/{id}/thumbnail` - Model thumbnail

### Slicing Operations

- `POST /api/slicer/slice` - Slice model with settings

### Printer Operations

- `GET /api/printers` - List printers
- `POST /api/printers/{id}/print` - Send print job

### Auth

- `GET /api/auth/status` - Check auth status
- Server handles auth via webview redirect

## User Interface

### Mobile Screens

1. **Server Setup** (first launch)
   - Server URL input
   - Save button
   - Test connection

2. **Model Library**
   - Grid view of models
   - Search bar
   - Pull to refresh

3. **Model Detail**
   - Thumbnail/preview
   - Info (name, size, format)
   - Actions: Slice, Send to Printer

4. **Slicing Settings**
   - Layer height dropdown
   - Infill slider
   - Supports toggle
   - Material dropdown

5. **Print Queue**
   - Active jobs
   - Job progress
   - History

### Platform-Specific UI

- **Mobile**: Bottom tab navigation, touch controls
- **Desktop**: Sidebar navigation, mouse/keyboard

## Configuration

### Environment Variables

- `VITE_API_PLATFORM`: "demo", "web", "tauri" (existing)
- Add: `VITE_MOBILE_SERVER_URL` - default remote server URL

### Storage

- Use Tauri secure storage for:
  - Server URL
  - Auth tokens (if applicable)
  - User preferences

## Dependencies

### Already Available

- `tauri-plugin-http` - for remote API calls
- `tauri-plugin-fs` - minimal file access
- `tauri-plugin-dialog` - native dialogs

### New Dependencies

- None required for basic functionality

## Build Commands

```bash
# Development
tauri android dev

# Release
tauri android build

# iOS (if needed)
tauri ios dev
tauri ios build
```

## Testing Strategy

### Unit Tests

- Platform detection logic
- API client functions
- UI state management

### Integration Tests

- End-to-end model import → slice → print
- Network error handling
- Auth flow

## Security Considerations

1. **HTTPS Only** - enforce TLS for server communication
2. **Token Storage** - use secure storage for auth tokens
3. **Input Validation** - validate server URL format
4. **CSP** - configure mobile-appropriate Content Security Policy

## Future Enhancements

1. Push notifications for print completion
2. Offline mode with sync
3. Multi-server support
4. Advanced slicing settings UI

---

_Implementation specification - to be updated as work progresses_
