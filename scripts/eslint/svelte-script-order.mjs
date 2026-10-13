// @ts-check
// mannie/svelte-script-order: every component script follows one order.
//
//     imports, types, props, state, derived values, functions, effects
//
// A reader then finds the same thing in the same place in every component.
// The order follows the examples of the Svelte documentation, with effects and
// lifecycle callbacks ($effect, onMount, beforeNavigate, ...) last because they
// work with everything declared above them.
//
// Statements outside these groups, such as a context lookup, a plain constant
// or a setContext call, may stand anywhere between the props and the effects:
// they often depend on state, or state depends on them.
//
// The rule reports and never moves code, because moving a statement can
// change what runs first. A helper that a $state or $derived initializer calls
// is a function declaration, which is hoisted, so it can stay in the functions
// group. The <script module> block is not checked.

const ORDER = ['imports', 'types', 'props', 'state', 'derived', 'functions', 'effects'];

/** @type {Record<string, { item: string, group: string }>} */
const NAMES = {
  derived: { group: 'derived values', item: '$derived declaration' },
  effects: { group: 'effects and lifecycle callbacks', item: 'effect or lifecycle callback' },
  functions: { group: 'functions', item: 'function' },
  imports: { group: 'imports', item: 'import' },
  other: { group: 'other statements', item: 'statement' },
  props: { group: 'props', item: '$props declaration' },
  state: { group: 'state', item: '$state declaration' },
  types: { group: 'type declarations', item: 'type declaration' },
};

/** @type {Record<string, string>} */
const CALL_GROUPS = {
  $derived: 'derived',
  '$derived.by': 'derived',
  $effect: 'effects',
  '$effect.pre': 'effects',
  '$effect.root': 'effects',
  $props: 'props',
  '$props.id': 'props',
  $state: 'state',
  '$state.raw': 'state',
  afterNavigate: 'effects',
  beforeNavigate: 'effects',
  onDestroy: 'effects',
  onMount: 'effects',
  onNavigate: 'effects',
};

const TYPE_DECLARATIONS = new Set(['TSEnumDeclaration', 'TSInterfaceDeclaration', 'TSTypeAliasDeclaration']);

const TYPESCRIPT_WRAPPERS = new Set(['TSAsExpression', 'TSNonNullExpression', 'TSSatisfiesExpression']);

/**
 * @param {any} node
 * @returns {any}
 */
function unwrapTypeScript(node) {
  let current = node;
  while (TYPESCRIPT_WRAPPERS.has(current.type)) {
    current = current.expression;
  }
  return current;
}

/**
 * The dotted name of a callee, such as `$state.raw`; empty for anything else.
 * @param {any} callee
 * @returns {string}
 */
function calleeName(callee) {
  const current = unwrapTypeScript(callee);
  if (current.type === 'Identifier') {
    return current.name;
  }
  if (current.type !== 'MemberExpression') {
    return '';
  }
  const hasNamedProperty = !current.computed && current.property.type === 'Identifier';
  if (!hasNamedProperty) {
    return '';
  }
  return `${calleeName(current.object)}.${current.property.name}`;
}

/**
 * The group of a rune or lifecycle call, or undefined.
 * @param {any} expression
 * @returns {string | undefined}
 */
function groupOfCall(expression) {
  const current = unwrapTypeScript(expression);
  if (current.type !== 'CallExpression') {
    return undefined;
  }
  const name = calleeName(current.callee);
  return Object.hasOwn(CALL_GROUPS, name) ? CALL_GROUPS[name] : undefined;
}

/**
 * @param {any} declaration A VariableDeclaration.
 * @returns {string}
 */
function groupOfVariable(declaration) {
  const initializer = declaration.declarations[0]?.init;
  if (!initializer) {
    return 'other';
  }
  const value = unwrapTypeScript(initializer);
  const isArrowFunction = value.type === 'ArrowFunctionExpression';
  const isFunctionExpression = value.type === 'FunctionExpression';
  if (isArrowFunction || isFunctionExpression) {
    return 'functions';
  }
  return groupOfCall(value) ?? 'other';
}

/**
 * @param {any} statement A top-level statement of the instance script.
 * @returns {string}
 */
function groupOf(statement) {
  const isExport = statement.type === 'ExportNamedDeclaration';
  const node = isExport && statement.declaration ? statement.declaration : statement;
  if (TYPE_DECLARATIONS.has(node.type)) {
    return 'types';
  }
  switch (node.type) {
    case 'ExpressionStatement': {
      return groupOfCall(node.expression) ?? 'other';
    }
    case 'FunctionDeclaration': {
      return 'functions';
    }
    case 'ImportDeclaration': {
      return 'imports';
    }
    case 'VariableDeclaration': {
      return groupOfVariable(node);
    }
    default: {
      return 'other';
    }
  }
}

/**
 * True for `<script module>` and the older `<script context="module">`.
 * @param {any} element A SvelteScriptElement.
 * @returns {boolean}
 */
function isModuleScript(element) {
  return element.startTag.attributes.some((/** @type {any} */ attribute) => {
    const name = attribute.key?.name;
    const isModuleFlag = name === 'module';
    const isContextAttribute = name === 'context';
    return isModuleFlag || isContextAttribute;
  });
}

/** @type {import('eslint').Rule.RuleModule} */
export const svelteScriptOrder = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Enforce one order of the statements in Svelte component scripts',
    },
    messages: {
      order:
        'Move this {{item}} above the {{group}}. A component script reads: imports, types, props, state, ' +
        'derived values, functions, effects.',
    },
    schema: [],
  },
  create(context) {
    const propsIndex = ORDER.indexOf('props');
    const effectsIndex = ORDER.indexOf('effects');

    /**
     * @param {any} statement
     * @param {string} group
     * @param {string} reachedGroup
     */
    function report(statement, group, reachedGroup) {
      context.report({
        data: { group: NAMES[reachedGroup].group, item: NAMES[group].item },
        messageId: 'order',
        node: statement,
      });
    }

    return {
      /** @param {any} element */
      SvelteScriptElement(element) {
        if (isModuleScript(element)) {
          return;
        }
        let reachedIndex = -1;
        let hasOtherStatements = false;
        for (const statement of element.body) {
          const group = groupOf(statement);
          if (group === 'other') {
            if (reachedIndex === effectsIndex) {
              report(statement, group, 'effects');
            }
            hasOtherStatements = true;
            continue;
          }
          const index = ORDER.indexOf(group);
          if (index < reachedIndex) {
            report(statement, group, ORDER[reachedIndex]);
            continue;
          }
          const isBeforeProps = index < propsIndex;
          if (hasOtherStatements && isBeforeProps) {
            report(statement, group, 'other');
            continue;
          }
          reachedIndex = index;
        }
      },
    };
  },
};
