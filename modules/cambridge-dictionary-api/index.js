const http = require('node:http');
const app = require('./data');
const port = Number(process.env.PORT || 3000);
const host = process.env.HOST || '127.0.0.1';
const server = http.createServer(app);
server.listen(port, host, () => {
  console.log(`Server running at http://${host}:${server.address().port}`);
  console.log('Try /api/dictionary/en-cn/hello');
});
server.on('error', (error) => {
  console.error(`Cannot start dictionary server: ${error.code || error.name}`);
  app.locals.close();
  process.exitCode = 1;
});
function stop() {
  server.close(() => { app.locals.close(); });
  setTimeout(() => { server.closeAllConnections(); app.locals.close(); }, 3000).unref();
}
process.once('SIGINT', stop);
process.once('SIGTERM', stop);
