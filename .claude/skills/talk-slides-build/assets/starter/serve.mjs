// Tiny static server for the deck. `npm run dev` runs this; export-pdf.mjs imports it.
// Aliases /vendor/* -> node_modules/reveal.js/* so index.html paths work in dev AND
// after `npm run build` (which copies reveal into public/vendor/).
import http from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL(".", import.meta.url));
const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".md": "text/markdown; charset=utf-8",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
  ".ttf": "font/ttf",
};

export function startServer({ port = 0, root = ROOT } = {}) {
  const server = http.createServer(async (req, res) => {
    try {
      let urlPath = decodeURIComponent(req.url.split("?")[0]);
      let filePath;
      if (urlPath.startsWith("/vendor/")) {
        filePath = join(root, "node_modules/reveal.js", urlPath.slice("/vendor/".length));
      } else {
        if (urlPath === "/") urlPath = "/index.html";
        filePath = join(root, normalize(urlPath).replace(/^(\.\.[/\\])+/, ""));
      }
      const data = await readFile(filePath);
      res.setHeader("Content-Type", MIME[extname(filePath)] || "application/octet-stream");
      res.end(data);
    } catch {
      res.statusCode = 404;
      res.end("Not found: " + req.url);
    }
  });
  return new Promise((resolve) =>
    server.listen(port, "127.0.0.1", () => resolve(server)),
  );
}

// Run directly?
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const port = Number(process.env.PORT) || 8000;
  startServer({ port }).then((s) => {
    console.log(`talk deck → http://127.0.0.1:${s.address().port}/   (Ctrl-C to stop)`);
    console.log(`presenter → add ?print-pdf for the print layout, or press S for speaker view`);
  });
}
