FROM node:20-alpine

WORKDIR /app

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# Copy package files
COPY package*.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile --prod

# Copy source code
COPY . .

# Build for production
RUN pnpm run build

# Expose port
EXPOSE 9435

# Start application
CMD ["pnpm", "run", "preview", "--", "--host", "0.0.0.0", "--port", "9435"]