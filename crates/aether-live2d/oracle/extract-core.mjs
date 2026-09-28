// Pull the Cubism Core (an Emscripten module) out of the "l2d" npm bundle,
// which embeds it, into a standalone script defining Live2DCubismCore.
import fs from 'node:fs';

const [bundle, out] = process.argv.slice(2);
const source = fs.readFileSync(bundle, 'utf8');
const marker = 'window.Live2DCubismCore = i, console.log = r;';
const end = source.indexOf(marker);
const start = source.lastIndexOf('(/* @__PURE__ */ e(((e, t) => {', end);
if (start < 0 || end < 0) throw new Error('Cubism Core not found in the bundle');
const tail = source.indexOf(')();', end) + 4;
fs.writeFileSync(
  out,
  'var e = (e, t) => () => (t || e((t = { exports: {} }).exports, t), t.exports);\n' +
    source.slice(start, tail) +
    '\nvar Live2DCubismCore = window.Live2DCubismCore;\n',
);
