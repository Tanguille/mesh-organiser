# Mesh Organiser NAS Deployment

This directory contains the Docker configuration for running Mesh Organiser on a NAS or any Docker-capable device.

## Prerequisites

- Docker installed and running
- Docker Compose installed (optional but recommended)
- Sufficient storage space for your model library
- Network access to the NAS from your mobile and web devices

## Quick Start

1. Copy this directory to your NAS
2. Navigate to the deployment directory
3. Start the service:

```bash
docker-compose up -d
```

The service will be available at `http://your-nas-ip:9435`

## Configuration

### Persistent Storage

The configuration uses two volume mounts:

- `./data:/app/data` - Stores your model library, thumbnails, and database
- `./config:/app/config` - Stores configuration files

Make sure these directories exist and have appropriate permissions.

### Environment Variables

- `NODE_ENV=production` - Runs the application in production mode
- `VITE_API_PLATFORM=web` - Configures the API platform (can be web, demo, or tauri)

## Customization

To customize the deployment:

1. Modify `docker-compose.yml` to change ports, volumes, or environment variables
2. Modify `Dockerfile` if you need to add additional dependencies or change the build process
3. Adjust resource limits in docker-compose.yml if needed for your NAS capabilities

## Maintenance

### Viewing Logs

```bash
docker-compose logs -f
```

### Stopping the Service

```bash
docker-compose down
```

### Updating the Service

```bash
docker-compose pull
docker-compose up -d
```

### Backing Up Your Data

Backup the `data` directory to preserve your model library and settings.

## Troubleshooting

### Service Not Starting

Check the logs:

```bash
docker-compose logs
```

### Port Already in Use

Either change the port in docker-compose.yml or stop the conflicting service.

### Performance Issues

Ensure your NAS has sufficient resources:

- Minimum: 2 CPU cores, 2GB RAM
- Recommended: 4+ CPU cores, 4GB+ RAM for smoother slicing performance

## Security Considerations

For external access:

- Consider setting up a reverse proxy with SSL/TLS
- Implement authentication if exposing to the internet
- Keep Docker and the container updated regularly

## Notes

This deployment runs the standard Mesh Organiser web instance, which provides:

- Model storage and organization
- Website importing (Thingiverse, Printables, etc.)
- Slicing via OrcaSlicer and other supported slicers
- Printer management and print job tracking
- REST API for Tauri desktop app

The Tauri desktop app connects to this instance via the standard API endpoints.
