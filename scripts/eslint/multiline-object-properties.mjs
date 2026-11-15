// @ts-check

/** @param {any} property An object literal property or spread. */
function hasInlineCallback(property) {
  const isValue = property.type === 'Property' && property.kind === 'init';
  const isFunction = /^(?:ArrowFunctionExpression|FunctionExpression)$/u.test(property.value?.type ?? '');
  return isValue && isFunction;
}

/** @type {import('eslint').Rule.RuleModule} */
export const multilineObjectProperties = {
  meta: {
    type: 'layout',
    fixable: 'whitespace',
    schema: [{ type: 'object', properties: { wrapCallbacks: { type: 'boolean' } }, additionalProperties: false }],
    messages: {
      newLine: 'Put each property of this multiline object on its own line.',
      wrapCallback: 'Put each property of this object with an inline callback on its own line.',
    },
  },

  create(context) {
    const { sourceCode } = context;
    const newline = sourceCode.text.match(/\r\n|\n|\r/u)?.[0] ?? '\n';
    const [options = {}] = context.options;

    /** @param {any} node An object literal. @param {import('eslint').Rule.RuleFixer} fixer */
    function wrapCallbackObject(node, fixer) {
      const line = sourceCode.lines[node.loc.start.line - 1];
      const indent = line.match(/^[\t ]*/u)?.[0] ?? '';
      const innerIndent = `${indent}  `;
      const fixes = [];

      for (const property of node.properties) {
        const separator = sourceCode.getTokenBefore(property, { includeComments: true });
        const isSeparator = separator?.type === 'Punctuator' && ['{', ','].includes(separator.value);
        if (!isSeparator) {
          return null;
        }

        fixes.push(fixer.replaceTextRange([separator.range[1], property.range[0]], `${newline}${innerIndent}`));
      }

      const close = sourceCode.getLastToken(node);
      const last = close && sourceCode.getTokenBefore(close, { includeComments: true });
      if (!close || !last) {
        return null;
      }

      fixes.push(fixer.replaceTextRange([last.range[1], close.range[0]], `${newline}${indent}`));
      return fixes;
    }

    return {
      ObjectExpression(node) {
        const { loc, properties } = node;
        if (loc.start.line === loc.end.line) {
          const hasSiblings = properties.length > 1;
          if (options.wrapCallbacks && hasSiblings && properties.some(hasInlineCallback)) {
            context.report({
              node,
              messageId: 'wrapCallback',
              fix: (fixer) => wrapCallbackObject(node, fixer),
            });
          }
          return;
        }

        for (let index = 1; index < properties.length; index += 1) {
          const previous = properties[index - 1];
          const property = properties[index];
          if (previous.loc.end.line !== property.loc.start.line) {
            continue;
          }

          context.report({
            node: property,
            messageId: 'newLine',
            fix(fixer) {
              const comma = sourceCode.getTokenBefore(property, { includeComments: true });
              if (comma?.value !== ',') {
                return null;
              }

              const previousLine = sourceCode.lines[previous.loc.start.line - 1];
              const indentation = previousLine.match(/^[\t ]*/u)?.[0] ?? '';
              const propertyIndent = previous.loc.start.line === loc.start.line ? `${indentation}  ` : indentation;

              return fixer.replaceTextRange([comma.range[1], property.range[0]], `${newline}${propertyIndent}`);
            },
          });
        }
      },
    };
  },
};
