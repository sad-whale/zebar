const assert = require('node:assert/strict');
const path = require('node:path');
const vm = require('node:vm');
const esbuild = require('../packages/client-api/node_modules/esbuild');

async function main() {
  const names = [
    'battery',
    'cpu',
    'date',
    'disk',
    'host',
    'ip',
    'keyboard',
    'memory',
    'network',
    'weather',
  ];
  const imports = names.map(name => {
    const symbol = `create${name[0].toUpperCase()}${name.slice(1)}Provider`;
    return `import { ${symbol} } from './src/providers/${name}/create-${name}-provider'; export { ${symbol} as ${name} };`;
  });
  const result = await esbuild.build({
    absWorkingDir: path.resolve(__dirname, '../packages/client-api'),
    stdin: {
      contents: imports.join('\n'),
      resolveDir: path.resolve(__dirname, '../packages/client-api'),
    },
    bundle: true,
    platform: 'node',
    format: 'cjs',
    write: false,
    plugins: [
      {
        name: 'platform-fixtures',
        setup(build) {
          build.onResolve({ filter: /^~\/(desktop|utils)$/ }, args => ({
            path: args.path,
            namespace: 'fixture',
          }));
          build.onLoad({ filter: /.*/, namespace: 'fixture' }, args => ({
            contents:
              args.path === '~/desktop'
                ? 'export const onProviderEmit = async () => () => {};'
                : 'export const createLogger = () => ({ debug(){}, warn(){} });',
          }));
        },
      },
    ],
  });
  const context = { module: { exports: {} } };
  vm.runInNewContext(result.outputFiles[0].text, context);
  for (const name of names) {
    for (const value of [0, -1, 0.5, NaN, Infinity, '']) {
      assert.throws(
        () =>
          context.module.exports[name]({
            type: name,
            refreshInterval: value,
          }),
        error => error.name === 'ZodError',
        `${name}: ${value}`,
      );
    }
    if (name !== 'date') {
      const provider = context.module.exports[name]({
        type: name,
        refreshInterval: '1000',
      });
      assert.equal(provider.config.refreshInterval, 1000);
      await provider.stop();
    }
  }
  console.log(
    'Provider intervals: 60 invalid configurations rejected; numeric-string coercion preserved.',
  );
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
