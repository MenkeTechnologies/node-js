// os: deterministic parts only — platform / arch / endianness / EOL / type.
const os = require("os");

console.log(os.platform());
console.log(os.arch());
console.log(os.endianness());
console.log(JSON.stringify(os.EOL));
console.log(os.type());
console.log(os.constants.signals.SIGINT);
console.log(os.constants.signals.SIGKILL);
console.log(os.constants.signals.SIGTERM);
console.log(typeof os.platform() === "string");
console.log(typeof os.totalmem() === "number");
console.log(["linux", "darwin", "win32", "freebsd", "openbsd", "aix", "sunos"].includes(os.platform()));
