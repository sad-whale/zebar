const assert = require('node:assert/strict');
const path = require('node:path');
const vm = require('node:vm');
const esbuild = require('../packages/client-api/node_modules/esbuild');

const events = Object.fromEntries(
  [
    'all',
    'binding_modes_changed',
    'focus_changed',
    'focused_container_moved',
    'tiling_direction_changed',
    'workspace_activated',
    'workspace_deactivated',
    'workspace_updated',
    'pause_changed',
    'window_managed',
    'window_unmanaged',
    'monitor_added',
    'monitor_removed',
    'monitor_updated',
    'user_config_changed',
  ].map(name => [name.toUpperCase(), name]),
);

async function main() {
  const result = await esbuild.build({
    absWorkingDir: path.resolve(__dirname, '../packages/client-api'),
    entryPoints: ['src/providers/glazewm/create-glazewm-provider.ts'],
    bundle: true,
    platform: 'node',
    format: 'cjs',
    write: false,
    plugins: [
      {
        name: 'platform-fixtures',
        setup(build) {
          build.onResolve(
            { filter: /^(glazewm|~\/desktop|~\/utils)$/ },
            args => ({
              path: args.path,
              namespace: 'fixture',
            }),
          );
          build.onLoad({ filter: /.*/, namespace: 'fixture' }, args => ({
            contents: {
              glazewm:
                'export const { WmClient, WmEventType } = globalThis.fixture;',
              '~/desktop':
                'export const getMonitors = async () => ({ currentMonitor: { x: 0, y: 0 } });',
              '~/utils':
                'export const getCoordinateDistance = (a, b) => Math.hypot(a.x-b.x, a.y-b.y); export const createLogger = () => ({debug(){}, warn(){}});',
            }[args.path],
          }));
        },
      },
    ],
  });

  let client;
  let monitorQueries = 0;
  let windowQueries = 0;
  let monitors = [
    {
      type: 'monitor',
      x: 0,
      y: 0,
      hasFocus: true,
      children: [
        {
          type: 'workspace',
          hasFocus: true,
          isDisplayed: true,
          children: [
            {
              type: 'split',
              children: [
                { type: 'window', id: 'tiled' },
                { type: 'window', id: 'minimized' },
              ],
            },
            { type: 'window', id: 'floating' },
          ],
        },
      ],
    },
  ];
  class WmClient {
    constructor() {
      client = this;
    }
    onDisconnect() {}
    onConnect(callback) {
      this.connect = callback;
    }
    async queryFocused() {
      return { focused: null };
    }
    async queryBindingModes() {
      return { bindingModes: [] };
    }
    async queryTilingDirection() {
      return { tilingDirection: 'horizontal' };
    }
    async queryPaused() {
      return { paused: false };
    }
    async queryMonitors() {
      monitorQueries++;
      return { monitors };
    }
    async queryWindows() {
      windowQueries++;
      return { windows: [] };
    }
    async subscribe(_, callback) {
      this.emit = callback;
      return () => {};
    }
    closeConnection() {}
  }
  const context = {
    module: { exports: {} },
    fixture: { WmClient, WmEventType: events },
  };
  vm.runInNewContext(result.outputFiles[0].text, context);
  const provider = context.module.exports.createGlazeWmProvider({
    type: 'glazewm',
  });
  let emissions = 0;
  provider.onOutput(() => emissions++);
  await new Promise(setImmediate);
  await client.connect();
  assert.deepEqual(
    Array.from(provider.output.allWindows, w => w.id),
    ['tiled', 'minimized', 'floating'],
  );
  assert.equal(monitorQueries, 1);
  assert.equal(
    windowQueries,
    0,
    'monitor trees already contain every window',
  );

  const before = emissions;
  await client.emit({ eventType: 'user_config_changed' });
  assert.equal(
    emissions,
    before,
    'irrelevant events must not redraw the bar',
  );
  await client.emit({ eventType: 'pause_changed', isPaused: true });
  assert.equal(provider.output.isPaused, true);
  assert.equal(
    monitorQueries,
    1,
    'pause changes do not need a monitor query',
  );

  monitors[0].children[0].children.push({ type: 'window', id: 'new' });
  await client.emit({ eventType: 'window_managed' });
  assert.equal(provider.output.allWindows.length, 4);
  monitors[0].children[0].children.pop();
  await client.emit({ eventType: 'window_unmanaged' });
  assert.equal(provider.output.allWindows.length, 3);
  assert.equal(windowQueries, 0);
  await provider.stop();
  console.log(
    'GlazeWM provider: nested windows, shared snapshots, event filtering and lifecycle checks passed.',
  );
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
