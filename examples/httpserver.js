// `http` request/response details. Nothing in the corpus reached http, and a
// server's own port is assigned by the OS, so nothing about the address is
// printed — only what travels over the connection.
const http = require("http");

const server = http.createServer((req, res) => {
  if (req.url === "/message") {
    // A custom reason phrase set as a PROPERTY. Only `statusCode` was being
    // read back off the response object, so this was dropped and the default
    // phrase for the code went out instead.
    res.statusCode = 201;
    res.statusMessage = "Created Custom";
    res.end("m");
    return;
  }
  if (req.url === "/writehead") {
    res.writeHead(202, "Head Message", { "X-Mixed-Case": "v" });
    res.end("w");
    return;
  }
  if (req.url === "/request") {
    // `rawHeaders` on the SERVER side: the flat name/value list in wire order
    // and case. The `headers` object is lower-cased and one-value-per-name, so
    // it can express neither; this was absent on both sides.
    const raw = req.rawHeaders;
    const idx = raw.indexOf("X-Client-Case");
    res.end([
      req.method,
      req.headers["x-client-case"],
      Array.isArray(raw),
      raw.length % 2 === 0,
      idx >= 0,
      idx >= 0 ? raw[idx + 1] : "-",
    ].join("|"));
    return;
  }
  res.end("d");
});

server.listen(0, () => {
  const port = server.address().port;
  const go = (path, opts, cb) => {
    const r = http.request({ port, path, ...opts }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (c) => { body += c; });
      res.on("end", () => cb(res, body));
    });
    r.end();
  };
  go("/message", {}, (res, body) => {
    console.log("message  ", res.statusCode, res.statusMessage, body);
    go("/writehead", {}, (res2, body2) => {
      console.log("writeHead", res2.statusCode, res2.statusMessage, body2);
      // The response's own rawHeaders keep the case the server wrote.
      console.log("raw-case ", res2.rawHeaders.includes("X-Mixed-Case"), res2.headers["x-mixed-case"]);
      console.log("raw-shape", Array.isArray(res2.rawHeaders), res2.rawHeaders.length % 2 === 0);
      go("/request", { method: "PUT", headers: { "X-Client-Case": "sent" } }, (res3, body3) => {
        console.log("request  ", body3);
        console.log("status   ", res3.statusCode, res3.statusMessage);
        server.close(() => console.log("closed   ", "yes"));
      });
    });
  });
});
