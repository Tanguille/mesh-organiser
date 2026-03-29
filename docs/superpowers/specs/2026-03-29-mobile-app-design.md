# Mobile and Web App for Mesh Organiser - Design Specification

## Overview

This document outlines the design for mobile and web companion apps to Mesh Organiser that provide model viewing, importing from supported websites, and basic slicing capabilities. The apps will be desktop-independent by leveraging a NAS-based Mesh Organiser web instance running in a container, which handles slicing via OrcaSlicer and provides the web UI for advanced features. Both mobile and web apps will offer comparable core functionality, with the mobile app focusing on Android initially (iOS may be added if it doesn't cause additional overhead).

## Core Features (Both Mobile and Web Apps)

### 1. Model Viewing and Organization

- Browse and view 3D models stored on the NAS-based Mesh Organiser instance
- Support for common 3D model formats (STL, OBJ, 3MF, STEP)
- Basic model information display (filename, size, date added)
- Thumbnail preview of models

### 2. Model Import from Websites

- Import models directly from supported websites (Thingiverse, Printables, Makerworld)
- Preserve existing redirect functionality from websites to the app
- Automatic grouping of imported .zip files by filename
- Integration with existing Mesh Organiser website support

### 3. Basic Slicing Capabilities

- Send models to NAS-based Mesh Organiser instance for slicing
- Configure basic slicing settings (layer height, infill percentage, supports)
- Trigger slicing via OrcaSlicer running on the NAS
- Receive and display sliced G-code preview
- Send sliced models to printer via existing printer integration

### 4. Printer Communication

- Send sliced G-code to configured printers
- Support for printers already configured in Mesh Organiser
- Basic print job management (start, pause, cancel)

## Architecture

### Client-Server Model

```
Mobile/Web App ↔ NAS Mesh Organiser (Web Instance in Container) ↔ Printers
                  ↓                                    ↓
          Model Requests                     Slicing via OrcaSlicer
                  ↓                                    ↓
              Sliced G-code                    Printer Communication
```

### Components

1. **Client Apps** (Mobile and Web)
   - **Mobile Client App** (React Native or similar, Android focus)
     - UI for model browsing, importing, and basic settings
     - Communication layer with NAS instance
     - Model viewing component (using existing thumbnails or generating previews)
     - Basic slicing settings UI
   - **Web Client App** (Existing mesh-organiser web)
     - Standard browser-based interface
     - Full feature set including advanced slicing and model management
     - Accessible via browser on any device

2. **NAS Mesh Organiser Instance** (Docker Container)
   - Running mesh-organiser web instance (serves both mobile and web clients)
   - Handles model storage and organization
   - Provides slicing functionality via OrcaSlicer integration
   - Exposes API for client app communication
   - Manages printer connections and print jobs

3. **OrcaSlicer Integration**
   - Leverages existing OrcaSlicer support in mesh-organiser
   - Uses NAS-based instance for slicing operations
   - Configured via standard mesh-organiser slicer settings

## Data Flow

### Model Import Flow

1. User selects model from supported website in mobile app
2. Mobile app sends import request to NAS mesh-organiser instance
3. NAS instance processes import (same as desktop/web flow)
4. Model stored with thumbnail generation
5. Mobile app receives confirmation and updates model list

### Slicing Flow

1. User selects model and adjusts basic settings in mobile app
2. Mobile app sends slicing request with model ID and settings to NAS instance
3. NAS instance:
   - Retrieves model from storage
   - Applies basic slicing settings
   - Invokes OrcaSlicer via existing slicer integration
   - Returns sliced G-code or 3MF file
4. Mobile app receives sliced file and displays preview
5. User can send to printer or adjust settings and reslice

### Printing Flow

1. User selects sliced model and printer in mobile app
2. Mobile app sends print job to NAS instance
3. NAS instance forwards to configured printer
4. Mobile app receives print job status updates

## Technical Implementation

### Client App Technologies

- **Mobile App** (React Native or similar, Android focus)
  - Framework: React Native (for cross-platform iOS/Android support)
  - State Management: Redux or Context API
  - Networking: Axios or Fetch for REST API calls
  - Model Viewing: Possibly use existing thumbnail generation or lightweight 3D viewer
  - Authentication: Token-based auth with NAS instance
- **Web App** (Existing mesh-organiser web)
  - Standard browser-based application
  - Built with SvelteKit (existing technology stack)
  - Accessible via any modern browser

### Shared NAS Instance Configuration

- Docker container running mesh-organiser web instance
- Standard mesh-organiser configuration with:
  - Enabled slicer support (including OrcaSlicer)
  - Configured data persistence via volume mounts
  - Network exposure for both mobile and web app access
  - Optional reverse proxy for HTTPS
  - Configured for multi-user support (existing feature)

### API Endpoints (Used by Both Apps)

Both mobile and web apps will use the existing mesh-organiser web APIs:

1. **Model Operations**
   - GET `/api/models` - List models
   - GET `/api/models/{id}` - Get model details
   - POST `/api/models` - Import model
   - DELETE `/api/models/{id}` - Delete model

2. **Slicing Operations**
   - POST `/api/slicer/slice` - Slice model with settings
   - Returns: Sliced G-code/3MF file and metadata

3. **Printer Operations**
   - GET `/api/printers` - List configured printers
   - POST `/api/printers/{id}/print` - Send print job
   - GET `/api/printers/{id}/status` - Get printer status

## User Interface (Comparable Functionality)

Both mobile and web apps will provide comparable core functionality with platform-appropriate interfaces:

### Main Screens (Both Apps)

1. **Model Library**
   - Grid/list view of models
   - Search and filter capabilities
   - Import button (+)
   - Sort options (name, date, etc.)

2. **Model Detail View**
   - Model thumbnail/preview
   - Model information (name, size, format)
   - Action buttons: Slice, Delete, Share

3. **Slicing Settings**
   - Basic settings panel:
     - Layer height (dropdown: 0.1mm, 0.2mm, 0.3mm)
     - Infill percentage (slider: 0-100%)
     - Supports (toggle: none, everywhere, touching buildplate)
     - Material type (dropdown: PLA, PETG, ABS, etc.)
   - Preview button to generate slice
   - Slice button to confirm and send to NAS

4. **Print Queue/Job Status**
   - Active print jobs
   - Job progress and controls
   - History of completed/failed jobs

### Platform-Specific Considerations

- **Mobile App**: Touch-optimized controls, native navigation patterns, offline capabilities
- **Web App**: Mouse/keyboard optimized, browser-native interface, potentially more advanced features

## Error Handling and Edge Cases

### Network Issues

- Queue requests when offline, sync when connection restored
- Clear error messages for connection failures
- Retry mechanisms for failed operations

### Slicing Failures

- Capture and display OrcaSlicer error messages
- Allow adjustment of settings and reslice
- Fallback to more detailed slicing UI via web interface if needed

### Storage Limitations

- Monitor NAS storage usage
- Warn user when approaching limits
- Provide cleanup options for old models/prints

## Security Considerations

### Authentication

- Secure authentication between mobile app and NAS instance
- Support for existing mesh-organiser user system
- Option for biometric/pin protection on mobile app

### Data Protection

- Encrypt sensitive data in transit (HTTPS)
- Secure storage of authentication tokens
- Regular security updates for NAS instance

## Testing Strategy

### Unit Testing

- Test mobile app components and state logic
- Test API request/response handling
- Test error conditions and edge cases

### Integration Testing

- Test end-to-end model import → slice → print flow
- Test with various model formats and sizes
- Test network interruption scenarios

### User Acceptance Testing

- Validate core workflows match user expectations
- Test usability of slicing settings interface
- Verify printer communication reliability

## Future Enhancements

### Advanced Slicing Features

- Support for custom OrcaSlicer profiles
- Multi-model plate arrangement
- Variable layer heights
- Custom support generation

### Enhanced Model Management

- Model tagging and categorization
- Batch operations (delete, slice multiple)
- Model versioning and history

### Social/Sharing Features

- Share models with other users
- Community model discovery
- Print settings sharing

## Dependencies and Requirements

### NAS Requirements

- Docker installed and running
- Sufficient storage for model library
- Adequate CPU/RAM for slicing operations (recommend 4+ cores, 4GB+ RAM)
- Network accessibility from mobile and web devices

### Client App Requirements

- **Mobile App**: Android 8+ (iOS may be added if it doesn't cause additional overhead)
- **Web App**: Any modern browser (Chrome, Firefox, Safari, Edge)
- Internet connectivity to NAS instance
- Optional: Biometric authentication support (mobile app only)

## Open Questions (Resolved)

1. Should the mobile app include its own minimal 3D model viewer, or rely on thumbnails from the NAS instance? -> **Include minimal 3D model viewer**
2. What level of slicing settings should be exposed in the mobile apps vs. requiring use of the full web interface? -> **Expose as much as possible in mobile app**
3. Should we implement push notifications for print job completion/failure? -> **No push notifications**
4. How should we handle authentication - extend existing mesh-organiser auth or implement mobile-specific tokens? -> **Use existing mesh-organiser authentication**
5. Should we prioritize any specific slicing settings or printer features for the initial mobile release? -> **No specific prioritization needed**

---

_Design approved and ready for implementation planning_
