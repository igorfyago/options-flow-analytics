FROM node:22-slim
WORKDIR /app
COPY api/package.json ./
RUN npm install --omit=dev
COPY api/ ./
EXPOSE 3000
CMD ["node", "server.js"]
