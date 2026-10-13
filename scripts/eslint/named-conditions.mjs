// @ts-check
// mannie/named-conditions: the parts of a compound condition have names.
//
// A condition that joins raw expressions with && or || makes the reader work
// out every part to learn what is checked. Named parts say it directly:
//
//     const isInRange = index < lines.length;
//     if (isInRange && isTableRow) { ... }
//
// Checked: the tests of if, while, do-while, for, ?: and Svelte's {#if}, and
// returned conditions (return statements and arrow function bodies whose
// parts compare or negate something). A part counts as named when it is a
// variable, a property path (`options.enabled`), a call of a named function or
// method with plain arguments (`isEmpty(items)`, `line.startsWith('|')`), or the
// negation of one of these. Comparisons, call chains and calls with computed
// arguments get a name first.
//
// Variable initializers are not checked, because `const isTableRow = a && b`
// already names the whole condition. That is also how to keep a guard whose
// later part is only safe after the earlier one, such as
// `index < lines.length && lines[index].startsWith('|')`: separate constants
// would evaluate the guarded part even when the guard fails.

const COMPARISON_OPERATORS = new Set(['==', '===', '!=', '!==', '<', '<=', '>', '>=', 'in', 'instanceof']);

const TYPESCRIPT_WRAPPERS = new Set(['TSAsExpression', 'TSNonNullExpression', 'TSSatisfiesExpression']);

/**
 * Removes TypeScript-only wrappers such as `value as T` and `value!`.
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
 * True for `!value`.
 * @param {any} node
 * @returns {boolean}
 */
function isNegation(node) {
  const isUnary = node.type === 'UnaryExpression';
  const isNot = node.operator === '!';
  return isUnary && isNot;
}

/**
 * True for `&&` and `||`, the operators that compose a condition.
 * @param {any} node
 * @returns {boolean}
 */
function isComposition(node) {
  const isLogical = node.type === 'LogicalExpression';
  const isNullishCoalescing = node.operator === '??';
  return isLogical && !isNullishCoalescing;
}

/**
 * A property path without calls: `name`, `this.items`, `lines[index]`, `user?.name`.
 * @param {any} node
 * @returns {boolean}
 */
function isPath(node) {
  const current = unwrapTypeScript(node);
  switch (current.type) {
    case 'Identifier':
    case 'MetaProperty':
    case 'Super':
    case 'ThisExpression': {
      return true;
    }
    case 'ChainExpression': {
      return isPath(current.expression);
    }
    case 'MemberExpression': {
      const property = current.property;
      const isPlainProperty = !current.computed || property.type === 'Identifier' || property.type === 'Literal';
      return isPlainProperty && isPath(current.object);
    }
    default: {
      return false;
    }
  }
}

/**
 * An argument that needs no explanation: a literal, a path or a negative number.
 * @param {any} node
 * @returns {boolean}
 */
function isPlainArgument(node) {
  const current = unwrapTypeScript(node);
  switch (current.type) {
    case 'Literal': {
      return true;
    }
    case 'TemplateLiteral': {
      return current.expressions.length === 0;
    }
    case 'UnaryExpression': {
      const isNegativeNumber = current.operator === '-' && current.argument.type === 'Literal';
      return isNegativeNumber;
    }
    default: {
      return isPath(current);
    }
  }
}

/**
 * True when a part of a condition reads as a name.
 * @param {any} node
 * @returns {boolean}
 */
function isNamed(node) {
  const current = unwrapTypeScript(node);
  switch (current.type) {
    case 'AwaitExpression': {
      return isNamed(current.argument);
    }
    case 'CallExpression': {
      return isPath(current.callee) && current.arguments.every(isPlainArgument);
    }
    case 'ChainExpression': {
      return isNamed(current.expression);
    }
    case 'Literal': {
      return true;
    }
    case 'UnaryExpression': {
      return isNegation(current) && isNamed(current.argument);
    }
    default: {
      return isPath(current);
    }
  }
}

/**
 * Collects the parts that && and || combine, looking through `!(...)`.
 * @param {any} node
 * @param {any[]} parts
 */
function collectParts(node, parts) {
  const current = unwrapTypeScript(node);
  if (isComposition(current)) {
    collectParts(current.left, parts);
    collectParts(current.right, parts);
    return;
  }
  const negatesComposition = isNegation(current) && isComposition(unwrapTypeScript(current.argument));
  if (negatesComposition) {
    collectParts(current.argument, parts);
    return;
  }
  parts.push(current);
}

/**
 * A returned && or || is a condition when one of its parts compares or negates.
 * Otherwise it usually picks a value, as in `cache.get(key) || load(key)`.
 * @param {any[]} parts
 * @returns {boolean}
 */
function looksLikeCondition(parts) {
  return parts.some((part) => {
    const isComparison = part.type === 'BinaryExpression' && COMPARISON_OPERATORS.has(part.operator);
    return isComparison || isNegation(part);
  });
}

/** @type {import('eslint').Rule.RuleModule} */
export const namedConditions = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Require the parts of a compound condition to be named values',
    },
    messages: {
      name:
        'Name this part of the condition (`const isInRange = …`) and combine the names. ' +
        'When the part is only safe after an earlier part, name the whole condition or move it into a function.',
    },
    schema: [],
  },
  create(context) {
    /**
     * @param {any} condition
     * @param {{ onlyConditions: boolean }} options
     */
    function check(condition, options) {
      let root = unwrapTypeScript(condition);
      while (isNegation(root)) {
        root = unwrapTypeScript(root.argument);
      }
      if (!isComposition(root)) {
        return;
      }
      /** @type {any[]} */
      const parts = [];
      collectParts(root, parts);
      if (options.onlyConditions && !looksLikeCondition(parts)) {
        return;
      }
      for (const part of parts) {
        if (!isNamed(part)) {
          context.report({ messageId: 'name', node: part });
        }
      }
    }

    /** @param {any} node */
    function checkTest(node) {
      if (node.test) {
        check(node.test, { onlyConditions: false });
      }
    }

    return {
      ArrowFunctionExpression(node) {
        if (node.expression) {
          check(node.body, { onlyConditions: true });
        }
      },
      ConditionalExpression: checkTest,
      DoWhileStatement: checkTest,
      ForStatement: checkTest,
      IfStatement: checkTest,
      ReturnStatement(node) {
        if (node.argument) {
          check(node.argument, { onlyConditions: true });
        }
      },
      /** @param {any} node */
      SvelteIfBlock(node) {
        check(node.expression, { onlyConditions: false });
      },
      WhileStatement: checkTest,
    };
  },
};
