/**
 * What the search contract does to its arguments before a provider sees them.
 *
 * The three providers answer with nothing in this phase, so nothing here can be tested
 * through them. What can be tested — and is the part that matters — is that `searchAll`
 * bounds what it passes on: in phase 03 the query stops being matched against eight titles
 * in the WebView and becomes an argument to a command in the core, and the ceiling has to
 * already be there when that happens rather than be remembered on the day.
 *
 * `contract.test-d.ts` covers the other half, which is a shape rather than a behaviour and
 * so is checked by the compiler instead of by this file.
 */

import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { MAX_HITS, MAX_QUERY, searchAll, type SearchHit, type SearchProvider } from './contract.ts';

/** A provider that answers nothing and writes down what it was asked. */
function recorder(module: SearchProvider['module']): {
  provider: SearchProvider;
  asked: { query: string; limit: number }[];
} {
  const asked: { query: string; limit: number }[] = [];

  return {
    asked,
    provider: {
      module,
      // Plain functions returning a promise rather than `async` ones: none of these has
      // anything to await, and the contract asks for a promise, not for an async function.
      search: (query, limit) => {
        asked.push({ query, limit });
        return Promise.resolve([]);
      },
    },
  };
}

/** A hit, for the cases about what comes back rather than what goes in. */
function hit(id: string): SearchHit {
  return { module: 'habits', id, title: `hábito ${id}`, occurredAt: null };
}

test('a query longer than the ceiling reaches the provider cut to it', async () => {
  const { provider, asked } = recorder('passwords');

  await searchAll([provider], 'a'.repeat(MAX_QUERY + 500), MAX_HITS);

  assert.equal(asked[0]?.query.length, MAX_QUERY);
});

test('a query within the ceiling is passed exactly as it was typed', async () => {
  // Including the spaces and the accents: folding is the palette's business, and a provider
  // that is handed something already mangled cannot match against what it stores.
  const { provider, asked } = recorder('habits');

  await searchAll([provider], '  Hábitos de Enero  ', MAX_HITS);

  assert.equal(asked[0]?.query, '  Hábitos de Enero  ');
});

test('a limit above the ceiling is brought down to it', async () => {
  const { provider, asked } = recorder('finances');

  await searchAll([provider], 'x', MAX_HITS + 1000);

  assert.equal(asked[0]?.limit, MAX_HITS);
});

test('a limit that is negative, fractional or not a number is made usable', async () => {
  const { provider, asked } = recorder('finances');

  await searchAll([provider], 'x', -5);
  await searchAll([provider], 'x', 7.9);
  await searchAll([provider], 'x', Number.NaN);

  assert.deepEqual(
    asked.map((each) => each.limit),
    [0, 7, MAX_HITS],
  );
});

test('a module that fails takes its own results with it and nothing else', async () => {
  const broken: SearchProvider = {
    module: 'passwords',
    search: () => Promise.reject(new Error('the vault closed underneath it')),
  };
  const working: SearchProvider = {
    module: 'habits',
    search: () => Promise.resolve([hit('1')]),
  };

  const found = await searchAll([broken, working], 'x', MAX_HITS);

  assert.deepEqual(
    found.map((each) => each.id),
    ['1'],
  );
});

test('more hits than the ceiling never reach the palette, however many providers there are', async () => {
  // The clamp is passed to each provider, and a provider that ignores it is still capped
  // here: three of them answering the maximum each would otherwise be 150 rows to draw.
  const generous = (module: SearchProvider['module']): SearchProvider => ({
    module,
    search: () =>
      Promise.resolve(
        Array.from({ length: MAX_HITS }, (_each, index) => hit(`${module}-${index}`)),
      ),
  });

  const found = await searchAll(
    [generous('habits'), generous('passwords'), generous('finances')],
    'x',
    MAX_HITS,
  );

  assert.equal(found.length, MAX_HITS);
});
