// @ts-check

const SIMPLE_ACTIONS = new Set(['AssignmentExpression', 'CallExpression', 'UpdateExpression']);
const INLINE_FUNCTIONS = new Set(['ArrowFunctionExpression', 'FunctionExpression']);
const BRANCHES = new Set(['ConditionalExpression', 'LogicalExpression']);
const NESTED_ACTIONS = new Set([
  ...INLINE_FUNCTIONS,
  'AssignmentExpression',
  'AwaitExpression',
  'SequenceExpression',
  'UpdateExpression',
  'YieldExpression',
]);
const EXPRESSION_WRAPPERS = new Set([
  'ChainExpression',
  'TSAsExpression',
  'TSInstantiationExpression',
  'TSNonNullExpression',
  'TSSatisfiesExpression',
  'TSTypeAssertion',
]);

/**
 * @param {any} node
 * @param {import('eslint').SourceCode.VisitorKeys} visitorKeys
 * @returns {any[]}
 */
function childrenOf(node, visitorKeys) {
  return (visitorKeys[node.type] ?? []).flatMap((key) => node[key]).filter(Boolean);
}

/**
 * @param {any} node
 * @returns {any}
 */
function unwrapExpression(node) {
  let expression = node;
  while (EXPRESSION_WRAPPERS.has(expression.type)) {
    expression = expression.expression;
  }
  return expression;
}

/**
 * @param {any} node
 * @param {import('eslint').SourceCode.VisitorKeys} visitorKeys
 * @param {boolean} inBranch
 * @returns {boolean}
 */
function hasNestedAction(node, visitorKeys, inBranch = false) {
  const isConditionalCall = inBranch && node.type === 'CallExpression';
  if (NESTED_ACTIONS.has(node.type) || isConditionalCall) {
    return true;
  }

  const childIsInBranch = inBranch || BRANCHES.has(node.type);
  return childrenOf(node, visitorKeys).some((child) => hasNestedAction(child, visitorKeys, childIsInBranch));
}

/**
 * @param {any} handler
 * @param {import('eslint').SourceCode.VisitorKeys} visitorKeys
 * @returns {boolean}
 */
function isSimpleHandler(handler, visitorKeys) {
  const isArrow = handler.type === 'ArrowFunctionExpression';
  if (!isArrow || handler.async) {
    return false;
  }

  const body = unwrapExpression(handler.body);
  const discardsResult = body.type === 'UnaryExpression' && body.operator === 'void';
  const action = discardsResult ? unwrapExpression(body.argument) : body;
  if (!SIMPLE_ACTIONS.has(action.type)) {
    return false;
  }

  const nestedExpressions = [...handler.params, ...childrenOf(action, visitorKeys)];
  return !nestedExpressions.some((node) => hasNestedAction(node, visitorKeys));
}

/** @type {import('eslint').Rule.RuleModule} */
export const noComplexInlineHandler = {
  meta: {
    type: 'suggestion',
    docs: { description: 'Keep inline Svelte event handlers to a single action' },
    schema: [],
    messages: { useNamedHandler: 'Move this handler into a named function or use a single simple action.' },
  },
  create(context) {
    const { visitorKeys } = context.sourceCode;

    /** @param {any} node */
    function checkHandler(node) {
      const expression = unwrapExpression(node);
      if (INLINE_FUNCTIONS.has(expression.type)) {
        if (!isSimpleHandler(expression, visitorKeys)) {
          context.report({ node: expression, messageId: 'useNamedHandler' });
        }
        return;
      }

      // An event attribute can select a handler or pass it through a wrapper.
      for (const child of childrenOf(expression, visitorKeys)) {
        checkHandler(child);
      }
    }

    return {
      /** @param {import('svelte-eslint-parser').AST.SvelteAttribute} node */
      SvelteAttribute(node) {
        // Mannie uses the `on` prefix for native handlers and component callbacks.
        if (!/^on[A-Za-z]/u.test(node.key.name)) {
          return;
        }
        for (const value of node.value) {
          if (value.type === 'SvelteMustacheTag') {
            checkHandler(value.expression);
          }
        }
      },
      /** @param {import('svelte-eslint-parser').AST.SvelteEventHandlerDirective} node */
      "SvelteDirective[kind='EventHandler']"(node) {
        if (node.expression) {
          checkHandler(node.expression);
        }
      },
    };
  },
};
