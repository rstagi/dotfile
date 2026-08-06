// Assemble a self-contained static site in public/ — publish it anywhere
// (GitHub Pages, Vercel, Netlify, `npx serve public`). index.html is copied
// unchanged because it already references vendor/ and theme/ relatively.
import { cp, rm, mkdir, access } from "node:fs/promises";

const u = (p) => new URL(p, import.meta.url);
const copyIfExists = async (from, to) => {
  try {
    await access(u(from));
    await cp(u(from), u(to), { recursive: true });
  } catch {}
};

await rm(u("./public"), { recursive: true, force: true });
await mkdir(u("./public"), { recursive: true });

await cp(u("./node_modules/reveal.js/dist"), u("./public/vendor/dist"), { recursive: true });
await cp(u("./node_modules/reveal.js/plugin"), u("./public/vendor/plugin"), { recursive: true });
await cp(u("./theme"), u("./public/theme"), { recursive: true });
await cp(u("./index.html"), u("./public/index.html"));
await copyIfExists("./slides.md", "./public/slides.md");
await copyIfExists("./slides", "./public/slides");
await copyIfExists("./assets", "./public/assets");
await copyIfExists("./images", "./public/images");

console.log("built static site → public/");
console.log("preview: npx serve public   |   deploy: point any static host at public/");
