const { join } = require('node:path');

// One cache directory for both phases. Puppeteer downloads its browser in a
// postinstall script and looks it up again at launch; when only the test
// script names the directory, the download lands somewhere else and the suite
// starts with "Could not find Chrome".
module.exports = {
  cacheDirectory: join(__dirname, '.puppeteer-cache'),
};
