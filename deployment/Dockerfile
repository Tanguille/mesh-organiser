FROM node:18-alpine

WORKDIR /app

# Copy package files
COPY package*.json ./
RUN npm ci --only=production

# Copy source code
COPY . .

# Build for production
RUN npm run build

# Expose port
EXPOSE 9435

# Start application
CMD ["npm", "start", "--", "--hostname", "0.0.0.0"]