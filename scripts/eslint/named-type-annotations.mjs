// @ts-check

const namedTypes = new Set(['TSInterfaceDeclaration', 'TSTypeAliasDeclaration']);

/** @type {import('eslint').Rule.RuleModule} */
export const namedTypeAnnotations = {
  meta: {
    type: 'suggestion',
    docs: { description: 'Use a named type for object shapes in TypeScript annotations' },
    messages: { nameType: 'Extract this object type into a named type alias or interface.' },
    schema: [],
  },
  create(context) {
    return {
      /** @param {any} node A TypeScript object type literal. */
      TSTypeLiteral(node) {
        let inAnnotation = false;
        for (let ancestor = node.parent; ancestor; ancestor = ancestor.parent) {
          // Moving an outer shape into a named type should keep its nested member shapes together.
          const hasOuterShape = ancestor.type === 'TSTypeLiteral';
          if (hasOuterShape || namedTypes.has(ancestor.type)) {
            return;
          }
          if (ancestor.type === 'TSTypeAnnotation') {
            inAnnotation = true;
          }
        }
        if (inAnnotation) {
          context.report({ node, messageId: 'nameType' });
        }
      },
    };
  },
};
