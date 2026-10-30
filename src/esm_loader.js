// The ES module loader: module records, linking and evaluation.
//
// Ported from ECMA-262 §16.2.1.5 (Cyclic Module Records) — Link /
// InnerModuleLinking, ResolveExport, GetExportedNames, GetModuleNamespace,
// Evaluate / InnerModuleEvaluation and the asynchronous-evaluation steps
// (ExecuteAsyncModule, GatherAvailableAncestors, AsyncModuleExecutionFulfilled
// and AsyncModuleExecutionRejected) — with the step structure kept so each
// function can be read against the spec. CommonJS, JSON and builtin modules
// are node's synthetic records: no environment of their own, a fixed list of
// export names, evaluated once.
//
// `N` holds the native half (src/esm.rs): resolution, parsing, compiling a
// lowered module body, CommonJS export-name detection and loading.
(function (N) {
  'use strict';

  /** url → module record. */
  const registry = new Map();
  /** The binding name ResolveExport reports for `export * as ns`. */
  const NAMESPACE = { namespace: true };
  const AMBIGUOUS = { ambiguous: true };
  /** Counter behind [[AsyncEvaluationOrder]]. */
  let asyncOrder = 0;

  function codedError(Ctor, code, message) {
    const e = new Ctor(message);
    e.code = code;
    return e;
  }

  // ── records ──────────────────────────────────────────────────────────────

  function newRecord(url, file, format) {
    const m = {
      url, file, format,
      cyclic: format === 'module',
      status: 'new',
      requested: [],
      namespace: undefined,
      hasError: false,
      error: undefined,
      // Cyclic Module Record fields.
      info: undefined,
      dfsIndex: -1,
      dfsAncestorIndex: -1,
      cycleRoot: undefined,
      asyncEvaluation: false,
      asyncEvaluationOrder: 0,
      topLevelCapability: undefined,
      asyncParentModules: [],
      pendingAsyncDependencies: 0,
      // The lowered body and its environment hooks.
      imp: undefined,
      meta: undefined,
      fn: undefined,
      gen: undefined,
      // What the body returned when it ran asynchronously: the promise an
      // unsettled top-level `await` leaves pending.
      body: undefined,
      getters: undefined,
      // Synthetic records: export names and, once evaluated, their values.
      exportNames: undefined,
      values: undefined,
      evaluated: false,
      isMain: false,
    };
    if (format === 'module') {
      m.info = N.parse(file);
      m.hasTLA = m.info.hasTLA;
    } else if (format === 'commonjs') {
      m.exportNames = ['default', 'module.exports'];
      for (const n of N.cjsNames(file)) {
        if (!m.exportNames.includes(n)) m.exportNames.push(n);
      }
    } else if (format === 'builtin') {
      m.builtin = N.builtin(url);
      m.exportNames = ['default'];
      for (const k of Object.keys(m.builtin)) {
        if (k !== 'default') m.exportNames.push(k);
      }
    } else {
      m.exportNames = ['default'];
    }
    return m;
  }

  /** The record for a resolution `[url, file, format]`, created on first use. */
  function recordFor(resolved, attributes) {
    const [url, file, format] = resolved;
    const type = attributes && attributes.type;
    if (type !== undefined && type !== 'json') {
      throw codedError(TypeError, 'ERR_IMPORT_ATTRIBUTE_UNSUPPORTED',
        `Import attribute "type" with value "${type}" is not supported`);
    }
    if (format === 'json' && type !== 'json') {
      throw codedError(TypeError, 'ERR_IMPORT_ATTRIBUTE_MISSING',
        `Module "${url}" needs an import attribute of "type: json"`);
    }
    if (type === 'json' && format !== 'json') {
      throw codedError(TypeError, 'ERR_IMPORT_ATTRIBUTE_TYPE_INCOMPATIBLE',
        `Module "${url}" is not of type "json"`);
    }
    let m = registry.get(url);
    if (m === undefined) {
      m = newRecord(url, file, format);
      registry.set(url, m);
    }
    return m;
  }

  /** Load every module `m` (transitively) requests: HostLoadImportedModule. */
  function loadGraph(m) {
    if (!m.cyclic || m.status !== 'new') return;
    const requests = m.info.requests;
    const requested = new Array(requests.length);
    for (let i = 0; i < requests.length; i++) {
      const [specifier, attributes] = requests[i];
      requested[i] = recordFor(N.resolve(specifier, m.file), attributes);
    }
    m.requested = requested;
    m.status = 'unlinked';
    for (const r of requested) loadGraph(r);
  }

  // ── export resolution (16.2.1.6.2, 16.2.1.6.3) ───────────────────────────

  function getExportedNames(m, exportStarSet) {
    if (!m.cyclic) return m.exportNames.slice();
    if (exportStarSet.includes(m)) return [];
    exportStarSet.push(m);
    const info = m.info;
    const names = [];
    for (const [exportName] of info.localExports) names.push(exportName);
    for (const [exportName] of info.indirectExports) names.push(exportName);
    for (const req of info.starExports) {
      for (const n of getExportedNames(m.requested[req], exportStarSet)) {
        if (n !== 'default' && !names.includes(n)) names.push(n);
      }
    }
    return names;
  }

  /** `{ module, bindingName }`, `null` (no such export) or AMBIGUOUS. */
  function resolveExport(m, exportName, resolveSet) {
    if (!m.cyclic) {
      // A builtin's export names are its own keys, which this runtime does
      // not enumerate completely for every core module (`process.argv` is
      // readable but not listed); a member that reads back defined is one.
      if (m.format === 'builtin' && !m.exportNames.includes(exportName)
        && m.builtin[exportName] !== undefined) {
        m.exportNames.push(exportName);
      }
      return m.exportNames.includes(exportName) ? { module: m, bindingName: exportName } : null;
    }
    for (const r of resolveSet) {
      // A circular import request.
      if (r.module === m && r.exportName === exportName) return null;
    }
    resolveSet.push({ module: m, exportName });
    const info = m.info;
    for (const [name, local] of info.localExports) {
      if (name === exportName) return { module: m, bindingName: local };
    }
    for (const [name, req, importName] of info.indirectExports) {
      if (name !== exportName) continue;
      const imported = m.requested[req];
      if (importName === null) return { module: imported, bindingName: NAMESPACE };
      return resolveExport(imported, importName, resolveSet);
    }
    if (exportName === 'default') return null;
    let starResolution = null;
    for (const req of info.starExports) {
      const resolution = resolveExport(m.requested[req], exportName, resolveSet);
      if (resolution === AMBIGUOUS) return AMBIGUOUS;
      if (resolution === null) continue;
      if (starResolution === null) {
        starResolution = resolution;
      } else if (resolution.module !== starResolution.module
        || resolution.bindingName !== starResolution.bindingName) {
        return AMBIGUOUS;
      }
    }
    return starResolution;
  }

  /** A reader of the binding `name` of `m`: the binding's current value. */
  function bindingReader(m, name, local) {
    if (name === NAMESPACE) return () => getNamespace(m);
    // A core module needs no evaluation step: its members are read from
    // the module object itself.
    if (m.format === 'builtin') return name === 'default' ? () => m.builtin : () => m.builtin[name];
    if (!m.cyclic) return () => (m.values === undefined ? undefined : m.values[name]);
    return () => {
      const g = m.getters === undefined ? undefined : m.getters.get(name);
      if (g === undefined) {
        throw new ReferenceError(`Cannot access '${local}' before initialization`);
      }
      return g();
    };
  }

  // ── module namespace exotic objects (10.4.6) ─────────────────────────────

  /** GetModuleNamespace: the namespace object of `m`, made once. */
  function getNamespace(m) {
    if (m.namespace !== undefined) return m.namespace;
    m.namespace = makeNamespace(namespaceEntries(m));
    return m.namespace;
  }

  /** `[exportName, reader]` for each unambiguous export of `m`. */
  function namespaceEntries(m) {
    const entries = [];
    for (const n of getExportedNames(m, [])) {
      const r = resolveExport(m, n, []);
      if (r !== null && r !== AMBIGUOUS) entries.push([n, bindingReader(r.module, r.bindingName, n)]);
    }
    return entries;
  }

  /** ModuleNamespaceCreate (10.4.6.12) over `[name, reader]` entries. */
  function makeNamespace(entries) {
    // [[Exports]] is sorted by code unit order.
    entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    const readers = new Map();
    const target = Object.create(null);
    for (const [n, read] of entries) {
      readers.set(n, read);
      Object.defineProperty(target, n, {
        value: undefined, writable: true, enumerable: true, configurable: false,
      });
    }
    Object.defineProperty(target, Symbol.toStringTag, { value: 'Module' });
    Object.preventExtensions(target);
    const isSym = (k) => typeof k === 'symbol';
    const ns = new Proxy(target, {
      get(t, k) {
        if (isSym(k)) return Reflect.get(t, k);
        const r = readers.get(k);
        return r === undefined ? undefined : r();
      },
      set() { return false; },
      has(t, k) { return isSym(k) ? Reflect.has(t, k) : readers.has(k); },
      deleteProperty(t, k) { return isSym(k) ? Reflect.deleteProperty(t, k) : !readers.has(k); },
      defineProperty(t, k, d) {
        if (isSym(k)) return Reflect.defineProperty(t, k, d);
        const r = readers.get(k);
        if (r === undefined) return false;
        if (d.configurable === true || d.enumerable === false || d.writable === false
          || 'get' in d || 'set' in d) return false;
        return 'value' in d ? Object.is(d.value, r()) : true;
      },
      getOwnPropertyDescriptor(t, k) {
        if (isSym(k)) return Reflect.getOwnPropertyDescriptor(t, k);
        const r = readers.get(k);
        if (r === undefined) return undefined;
        return { value: r(), writable: true, enumerable: true, configurable: false };
      },
      setPrototypeOf(t, p) { return p === null; },
      preventExtensions() { return true; },
    });
    N.markNamespace(ns);
    return ns;
  }

  // ── linking (16.2.1.5.1) ─────────────────────────────────────────────────

  function link(m) {
    const stack = [];
    try {
      innerModuleLinking(m, stack, 0);
    } catch (e) {
      for (const s of stack) s.status = 'unlinked';
      throw e;
    }
  }

  function innerModuleLinking(m, stack, index) {
    if (!m.cyclic) return index;
    if (m.status !== 'unlinked') return index;
    m.status = 'linking';
    m.dfsIndex = index;
    m.dfsAncestorIndex = index;
    index++;
    stack.push(m);
    for (const required of m.requested) {
      index = innerModuleLinking(required, stack, index);
      if (required.cyclic && required.status === 'linking') {
        m.dfsAncestorIndex = Math.min(m.dfsAncestorIndex, required.dfsAncestorIndex);
      }
    }
    initializeEnvironment(m);
    if (m.dfsAncestorIndex === m.dfsIndex) {
      const component = [];
      let done = false;
      while (!done) {
        const required = stack.pop();
        required.status = 'linked';
        component.push(required);
        done = required === m;
      }
      // A module in an import cycle must have its environment before any
      // module of the cycle evaluates; it is built as a generator whose
      // prologue runs now. An acyclic module is instantiated when it runs.
      const cyclic = component.length > 1 || m.requested.includes(m);
      for (const c of component) {
        c.fn = N.compile(c.info.id, cyclic);
        if (cyclic) {
          c.gen = c.fn.call(undefined, c.imp, c.meta, (getters) => register(c, getters));
          c.gen.next();
        }
      }
    }
    return index;
  }

  function register(m, getters) {
    const map = new Map();
    const locals = m.info.localExports;
    for (let i = 0; i < getters.length; i++) map.set(locals[i][1], getters[i]);
    m.getters = map;
  }

  function missingExport(m, req, name, clause) {
    const specifier = m.info.requests[req][0];
    const imported = m.requested[req];
    if (imported.format === 'commonjs') {
      return new SyntaxError(`Named export '${name}' not found. The requested module '${specifier}' `
        + 'is a CommonJS module, which may not support all module.exports as named exports.\n'
        + 'CommonJS modules can always be imported via the default export, for example using:\n\n'
        + `import pkg from '${specifier}';\nconst ${clause || `{${name}}`} = pkg;\n`);
    }
    return new SyntaxError(`The requested module '${specifier}' does not provide an export named '${name}'`);
  }

  function checkResolution(m, req, name, r, clause) {
    if (r === null) throw missingExport(m, req, name, clause);
    if (r === AMBIGUOUS) {
      throw new SyntaxError(`The requested module '${m.info.requests[req][0]}' `
        + `contains conflicting star exports for name '${name}'`);
    }
  }

  /** InitializeEnvironment (16.2.1.6.4): resolve every import binding. */
  function initializeEnvironment(m) {
    const info = m.info;
    for (const [exportName, req, importName] of info.indirectExports) {
      if (importName === null) continue;
      checkResolution(m, req, importName, resolveExport(m, exportName, []));
    }
    const imp = new Array(info.imports.length);
    info.imports.forEach(([req, name, local, clause], k) => {
      const imported = m.requested[req];
      if (name === null) {
        const ns = getNamespace(imported);
        imp[k] = () => ns;
        return;
      }
      const r = resolveExport(imported, name, []);
      checkResolution(m, req, name, r, clause);
      if (r.bindingName === NAMESPACE) {
        const ns = getNamespace(r.module);
        imp[k] = () => ns;
        return;
      }
      // Resolved once, on first read, then the getter itself takes its slot:
      // later reads are one call.
      const read = bindingReader(r.module, r.bindingName, local);
      imp[k] = function () {
        const value = read();
        if (r.module.cyclic) imp[k] = r.module.getters.get(r.bindingName);
        return value;
      };
    });
    m.imp = imp;
    m.meta = createMeta(m);
  }

  function createMeta(m) {
    const meta = Object.create(null);
    const slash = m.file.lastIndexOf('/');
    meta.dirname = slash > 0 ? m.file.slice(0, slash) : '/';
    meta.filename = m.file;
    meta.main = m.isMain;
    meta.resolve = function resolve(specifier) {
      return N.resolve(String(specifier), m.file)[0];
    };
    meta.url = m.url;
    return meta;
  }

  // ── evaluation (16.2.1.5.3) ──────────────────────────────────────────────

  function capability() {
    let resolve;
    let reject;
    const promise = new Promise((a, b) => { resolve = a; reject = b; });
    return { promise, resolve, reject };
  }

  function evaluate(m) {
    // A module whose evaluation threw before its component finished has no
    // [[CycleRoot]]; its own capability already carries the error.
    if (m.cyclic && (m.status === 'evaluating-async' || m.status === 'evaluated')
      && m.cycleRoot !== undefined) {
      m = m.cycleRoot;
    }
    if (m.topLevelCapability !== undefined) return m.topLevelCapability.promise;
    const stack = [];
    const cap = capability();
    m.topLevelCapability = cap;
    try {
      innerModuleEvaluation(m, stack, 0);
      if (!m.cyclic || !m.asyncEvaluation) cap.resolve(undefined);
    } catch (e) {
      for (const s of stack) {
        s.status = 'evaluated';
        s.hasError = true;
        s.error = e;
      }
      if (!m.cyclic) {
        m.hasError = true;
        m.error = e;
      }
      cap.reject(e);
    }
    return cap.promise;
  }

  function evaluateSynthetic(m) {
    if (m.evaluated) {
      if (m.hasError) throw m.error;
      return;
    }
    m.evaluated = true;
    try {
      const values = Object.create(null);
      if (m.format === 'commonjs') {
        const exports = N.requireCjs(m.file);
        values.default = exports;
        values['module.exports'] = exports;
        for (const n of m.exportNames.slice(2)) {
          values[n] = exports === null || exports === undefined ? undefined : exports[n];
        }
      } else if (m.format === 'json') {
        values.default = N.json(m.file);
      }
      m.values = values;
    } catch (e) {
      m.hasError = true;
      m.error = e;
      throw e;
    }
  }

  function innerModuleEvaluation(m, stack, index) {
    if (!m.cyclic) {
      evaluateSynthetic(m);
      return index;
    }
    if (m.status === 'evaluating-async' || m.status === 'evaluated') {
      if (m.hasError) throw m.error;
      return index;
    }
    if (m.status === 'evaluating') return index;
    m.status = 'evaluating';
    m.dfsIndex = index;
    m.dfsAncestorIndex = index;
    m.pendingAsyncDependencies = 0;
    index++;
    stack.push(m);
    for (let required of m.requested) {
      index = innerModuleEvaluation(required, stack, index);
      if (!required.cyclic) continue;
      if (required.status === 'evaluating') {
        m.dfsAncestorIndex = Math.min(m.dfsAncestorIndex, required.dfsAncestorIndex);
      } else {
        required = required.cycleRoot;
        if (required.hasError) throw required.error;
      }
      if (required.asyncEvaluation) {
        m.pendingAsyncDependencies++;
        required.asyncParentModules.push(m);
      }
    }
    if (m.pendingAsyncDependencies > 0 || m.hasTLA) {
      m.asyncEvaluation = true;
      m.asyncEvaluationOrder = ++asyncOrder;
      if (m.pendingAsyncDependencies === 0) executeAsyncModule(m);
    } else {
      executeModule(m);
    }
    if (m.dfsAncestorIndex === m.dfsIndex) {
      let done = false;
      while (!done) {
        const required = stack.pop();
        required.status = required.asyncEvaluation ? 'evaluating-async' : 'evaluated';
        required.cycleRoot = m;
        done = required === m;
      }
    }
    return index;
  }

  /** ExecuteModule: run the body (a promise for a module with TLA). */
  function executeModule(m) {
    if (m.gen !== undefined) return m.gen.next();
    return m.fn.call(undefined, m.imp, m.meta, (getters) => register(m, getters));
  }

  function executeAsyncModule(m) {
    m.body = executeModule(m);
    Promise.resolve(m.body).then(
      () => asyncModuleExecutionFulfilled(m),
      (e) => asyncModuleExecutionRejected(m, e),
    );
  }

  function gatherAvailableAncestors(m, execList) {
    for (const p of m.asyncParentModules) {
      if (!execList.includes(p) && !p.cycleRoot.hasError) {
        p.pendingAsyncDependencies--;
        if (p.pendingAsyncDependencies === 0) {
          execList.push(p);
          if (!p.hasTLA) gatherAvailableAncestors(p, execList);
        }
      }
    }
  }

  function asyncModuleExecutionFulfilled(m) {
    if (m.status === 'evaluated') return;
    m.asyncEvaluation = false;
    m.status = 'evaluated';
    if (m.topLevelCapability !== undefined) m.topLevelCapability.resolve(undefined);
    const execList = [];
    gatherAvailableAncestors(m, execList);
    execList.sort((a, b) => a.asyncEvaluationOrder - b.asyncEvaluationOrder);
    for (const p of execList) {
      if (p.status === 'evaluated') continue;
      if (p.hasTLA) {
        executeAsyncModule(p);
        continue;
      }
      try {
        executeModule(p);
      } catch (e) {
        asyncModuleExecutionRejected(p, e);
        continue;
      }
      p.asyncEvaluation = false;
      p.status = 'evaluated';
      if (p.topLevelCapability !== undefined) p.topLevelCapability.resolve(undefined);
    }
  }

  function asyncModuleExecutionRejected(m, error) {
    if (m.status === 'evaluated') return;
    m.hasError = true;
    m.error = error;
    m.status = 'evaluated';
    for (const p of m.asyncParentModules) asyncModuleExecutionRejected(p, error);
    if (m.topLevelCapability !== undefined) m.topLevelCapability.reject(error);
  }

  // ── host entry points ────────────────────────────────────────────────────

  /** A thrown value re-raised as an uncaught exception, out of any promise. */
  function rethrowUncaught(e) {
    queueMicrotask(() => { throw e; });
  }

  /** `node entry.mjs`: load and link now, evaluate from a microtask, as
   *  node's asynchronous loader does. */
  function runMain(file, url, isMain) {
    const m = recordFor([url, file, 'module'], undefined);
    mainModule = m;
    m.isMain = isMain;
    loadGraph(m);
    link(m);
    mainSettled = false;
    queueMicrotask(() => {
      const p = evaluate(m);
      p.catch(() => {});
      if (m.status === 'evaluated' && m.hasError) {
        mainSettled = true;
        throw m.error;
      }
      p.then(() => { mainSettled = true; }, (e) => {
        mainSettled = true;
        rethrowUncaught(e);
      });
    });
  }

  /** The entry module's evaluation, when the loop drains with it still
   *  waiting on a top-level `await` (node's exit status 13): `[url, body]` for
   *  each module of its graph that is itself suspended in one, found as V8's
   *  SourceTextModule::InnerGetStalledTopLevelAwaitModule finds them — a
   *  module still evaluating asynchronously with no async dependency left
   *  pending is stalled in its own body; any other is searched through its
   *  requested modules, each visited once. `undefined` once it has settled. */
  let mainSettled = true;
  let mainModule;
  function mainUnsettled() {
    if (mainSettled) return undefined;
    const stalled = [];
    const visited = new Set();
    const search = (m) => {
      if (m.pendingAsyncDependencies === 0 && m.asyncEvaluation) {
        stalled.push([m.url, m.body]);
        return;
      }
      for (const r of m.requested) {
        if (r.cyclic && !visited.has(r)) {
          visited.add(r);
          search(r);
        }
      }
    };
    search(mainModule);
    return stalled;
  }

  function attributesOf(options) {
    if (options === undefined) return undefined;
    if (options === null || typeof options !== 'object') {
      throw new TypeError('The second argument to import() must be an object');
    }
    const attrs = options.with === undefined ? options.assert : options.with;
    if (attrs === undefined) return undefined;
    if (attrs === null || typeof attrs !== 'object') {
      throw new TypeError("The 'with' option must be an object");
    }
    return attrs;
  }

  /** `import(specifier, options)` from the module or script file `referrer`. */
  function dynamicImport(specifier, referrer, options) {
    return Promise.resolve().then(() => {
      const attrs = attributesOf(options);
      const from = typeof referrer === 'string' && referrer.startsWith('/') ? referrer : undefined;
      const m = recordFor(N.resolve(String(specifier), from), attrs);
      loadGraph(m);
      link(m);
      return evaluate(m).then(() => getNamespace(m));
    });
  }

  function graphHasTLA(m, seen) {
    if (!m.cyclic || seen.has(m)) return false;
    seen.add(m);
    return m.hasTLA || m.requested.some((r) => graphHasTLA(r, seen));
  }

  /** `require(esm)`: a synchronous graph only, yielding its namespace (or
   *  the value it exports as `"module.exports"`). */
  function requireModule(file, parent) {
    const resolved = N.resolve(file, undefined);
    const m = recordFor([resolved[0], resolved[1], 'module'], undefined);
    loadGraph(m);
    link(m);
    if (graphHasTLA(m, new Set())) {
      throw codedError(Error, 'ERR_REQUIRE_ASYNC_MODULE',
        'require() cannot be used on an ESM graph with top-level await. Use import() instead. '
        + 'To see where the top-level await comes from, use --experimental-print-required-tla.\n'
        + `Required module: ${m.file}` + (parent ? `\nRequire stack:\n- ${parent}` : ''));
    }
    evaluate(m).catch(() => {});
    const root = m.cycleRoot === undefined ? m : m.cycleRoot;
    if (root.hasError) throw root.error;
    const names = getExportedNames(m, []);
    if (names.includes('module.exports')) return getNamespace(m)['module.exports'];
    // A module with a default export gets `__esModule: true` added, so code
    // compiled from ESM to CommonJS recognises the default (node's rule).
    if (names.includes('default') && !names.includes('__esModule')) {
      if (m.requireFacade === undefined) {
        m.requireFacade = makeNamespace([...namespaceEntries(m), ['__esModule', () => true]]);
      }
      return m.requireFacade;
    }
    return getNamespace(m);
  }

  return { runMain, dynamicImport, requireModule, mainUnsettled };
})
