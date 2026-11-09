const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const ts = require('../packages/desktop/node_modules/typescript');

const wifi = {
  type: 'wifi',
  ipv4Addresses: ['192.0.2.2/24'],
  ipv6Addresses: [],
};
const tunnel = {
  type: 'tunnel',
  name: 'utun4',
  ipv4Addresses: ['100.64.0.1/32'],
};
const fixtures = [
  [
    'VPN with physical Wi-Fi',
    {
      defaultInterface: { ...tunnel, type: 'unknown' },
      interfaces: [tunnel, wifi],
    },
    'nf-md-wifi',
  ],
  [
    'IPv6-only Wi-Fi under VPN',
    {
      defaultInterface: tunnel,
      interfaces: [
        { ...wifi, ipv4Addresses: [], ipv6Addresses: ['2001:db8::2/64'] },
      ],
    },
    'nf-md-wifi',
  ],
  [
    'link-local alone is not an active Wi-Fi uplink',
    {
      defaultInterface: tunnel,
      interfaces: [
        { ...wifi, ipv4Addresses: [], ipv6Addresses: ['fe80::2/64'] },
      ],
    },
    'nf-md-shield_lock_outline',
  ],
  [
    'unknown route is not Wi-Fi off',
    { defaultInterface: { type: 'unknown' }, interfaces: [] },
    'nf-md-network_outline',
  ],
  [
    'unknown Wi-Fi strength',
    {
      defaultInterface: wifi,
      interfaces: [wifi],
      defaultGateway: { signalStrength: null },
    },
    'nf-md-wifi',
  ],
  [
    'known Wi-Fi strength',
    {
      defaultInterface: wifi,
      interfaces: [wifi],
      defaultGateway: { signalStrength: 85 },
    },
    'nf-md-wifi_strength_4',
  ],
  [
    'Ethernet keeps its icon',
    { defaultInterface: { type: 'ethernet' }, interfaces: [wifi] },
    'nf-md-ethernet_cable',
  ],
  [
    'no active connection',
    { defaultInterface: null, interfaces: [] },
    'nf-md-wifi_strength_off_outline',
  ],
];
const paths = process.argv.slice(2);
if (!paths.length)
  paths.push(
    ...['vanilla', 'with-glazewm', 'with-komorebi'].map(
      n => `resources/starter/${n}.html`,
    ),
  );
for (const path of paths) {
  const html = fs.readFileSync(path, 'utf8');
  const start = html.search(/function (?:getNetworkIcon|networkIcon)\(/);
  assert(start >= 0, `${path}: network icon function missing`);
  const next = html.indexOf('function ', start + 9);
  let source = html
    .slice(start, next)
    .replace(/\s*\/\/ Get icon to show[\s\S]*$/, '');
  const name = source.match(/function (\w+)/)[1];
  const context = {
    React: { createElement: (_, props) => props.className },
  };
  const compiled = ts.transpileModule(source, {
    compilerOptions: {
      jsx: ts.JsxEmit.React,
      target: ts.ScriptTarget.ES2020,
    },
  });
  const icon = vm.runInNewContext(
    `${compiled.outputText}; ${name}`,
    context,
  );
  for (const [description, input, expected] of fixtures) {
    const actual = icon(input).split(' ').at(-1);
    assert.equal(actual, expected, `${path}: ${description}`);
  }
  console.log(`PASS ${path}: ${fixtures.length} network cases`);
}
