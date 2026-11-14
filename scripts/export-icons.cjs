// Export lucide-react icons as the SVG files in assets/icons:
//   node export-icons.cjs <frontend dir> <out dir> <kebab-name> ...
// React's static markup of the icon, root stroke currentColor -> #000000
// (tinted by iced's colour filter), trailing newline.
const path = require('path');
const fs = require('fs');
const fe = process.argv[2], out = process.argv[3];
const req = (m) => require(require.resolve(m, { paths: [fe] }));
const React = req('react');
const { renderToStaticMarkup } = req('react-dom/server');
const lucide = req('lucide-react');
for (const file of process.argv.slice(4)) {
  const name = file.split('-').map((p) => p[0].toUpperCase() + p.slice(1)).join('');
  const Icon = lucide[name];
  if (!Icon) throw new Error('no icon ' + name);
  let s = renderToStaticMarkup(React.createElement(Icon));
  s = s.replace('stroke="currentColor"', 'stroke="#000000"').replace(/ class="[^"]*"/, '');
  fs.writeFileSync(path.join(out, file + '.svg'), s + '\n');
}
