const pkg = require("../package.json");
exports.version = pkg.version;
exports.typescriptVersion = pkg.tsrs.typescriptVersion;
exports.typescriptCommit = pkg.tsrs.typescriptCommit;
exports.versionMajorMinor = /^\d+\.\d+/.exec(pkg.tsrs.typescriptVersion)[0];
