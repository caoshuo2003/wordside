const axios = require('axios');
const http = require('node:http');
const https = require('node:https');

// The upstream User-Agent and 10-second request timeout are retained.
// One client and its agents are reused for all dictionary lookups.
function createHttpClient() {
  const agents = { keepAlive: true, maxSockets: 8, maxFreeSockets: 4, timeout: 30000 };
  const httpAgent = new http.Agent(agents);
  const httpsAgent = new https.Agent(agents);
  const client = axios.create({
    timeout: 10000,
    maxContentLength: 4 * 1024 * 1024,
    maxRedirects: 5,
    responseType: 'text',
    transformResponse: [(value) => value],
    httpAgent,
    httpsAgent,
    headers: {
      'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
    },
    beforeRedirect(options) {
      // Axios applies proxy settings before this callback. href retains the
      // destination URL even when hostname/protocol now describe the proxy.
      const destination = new URL(options.href || `${options.protocol}//${options.hostname}${options.path}`);
      if (destination.protocol !== 'https:' || destination.hostname !== 'dictionary.cambridge.org' || !['', '443'].includes(destination.port)) {
        throw new Error('Unexpected Cambridge redirect');
      }
    },
  });
  client.close = () => { httpAgent.destroy(); httpsAgent.destroy(); };
  return client;
}
module.exports = { createHttpClient };
