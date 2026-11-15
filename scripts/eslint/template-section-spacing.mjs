// @ts-check
import { elementName, hasWhitespaceSensitiveContext, isWhitespace } from './template-newlines.mjs';

const SECTION_BLOCK = /^Svelte(?:Await|Each|If|Key|Snippet)Block$/u;
const REPEATED_ITEMS = new Set(['dd', 'dt', 'li', 'optgroup', 'option', 'td', 'th', 'tr']);
const SIBLING_CONTAINERS = [
  'SvelteElement',
  'SvelteIfBlock',
  'SvelteElseBlock',
  'SvelteEachBlock',
  'SvelteAwaitPendingBlock',
  'SvelteAwaitThenBlock',
  'SvelteAwaitCatchBlock',
  'SvelteKeyBlock',
  'SvelteSnippetBlock',
].join(', ');

/** @param {any} node A template child. */
function isSection(node) {
  if (node.type === 'SvelteElement') {
    // A wrapped attribute list still belongs to a single compact control.
    const openingLine = node.startTag.loc.end.line;
    return node.children.some((/** @type {any} */ child) => {
      const rendersMarkup = /^(?:SvelteElement|SvelteRenderTag)$/u.test(child.type) || SECTION_BLOCK.test(child.type);
      const startsOnOwnLine = child.loc.start.line > openingLine;
      return rendersMarkup && startsOnOwnLine;
    });
  }

  const multiline = node.loc.start.line < node.loc.end.line;
  return SECTION_BLOCK.test(node.type) && multiline;
}

/** @param {any} program A Svelte program. */
function hasPreserveWhitespaceOption(program) {
  return program.body.some((/** @type {any} */ node) => {
    if (elementName(node) !== 'svelte:options') {
      return false;
    }

    return node.startTag.attributes.some((/** @type {any} */ attribute) => {
      const isWhitespaceOption = attribute.key?.name === 'preserveWhitespace';
      const isEnabled = attribute.value?.[0]?.expression?.value !== false;
      return isWhitespaceOption && isEnabled;
    });
  });
}

/** @type {import('eslint').Rule.RuleModule} */
export const templateSectionSpacing = {
  meta: {
    type: 'layout',
    docs: { description: 'Separate sibling Svelte template sections with a blank line' },
    fixable: 'whitespace',
    messages: { blankLine: 'Put a blank line between these template sections.' },
    schema: [],
  },
  create(context) {
    const { sourceCode } = context;
    let preserveWhitespace = Boolean(
      context.languageOptions.parserOptions?.svelteConfig?.compilerOptions?.preserveWhitespace,
    );

    /** @param {any} container The parent of a markup sibling list. @param {any[]} children */
    function checkSiblings(container, children) {
      if (preserveWhitespace) {
        return;
      }

      const siblings = children.filter((child) => !isWhitespace(child));
      const hasAdjacentSiblings = siblings.length > 1;
      if (!hasAdjacentSiblings || hasWhitespaceSensitiveContext(container, sourceCode)) {
        return;
      }

      for (let index = 1; index < siblings.length; index += 1) {
        const previous = siblings[index - 1];
        const current = siblings[index];
        const repeatedItems = REPEATED_ITEMS.has(elementName(previous)) && REPEATED_ITEMS.has(elementName(current));
        if (repeatedItems || !isSection(previous) || !isSection(current)) {
          continue;
        }

        /** @type {[number, number]} */
        const range = [previous.range[1], current.range[0]];
        const gap = sourceCode.text.slice(...range);
        const newline = /^[\t ]*(\r\n|\n|\r)[\t ]*$/u.exec(gap)?.[1];
        if (!newline) {
          continue;
        }

        context.report({
          loc: current.loc,
          messageId: 'blankLine',
          fix: (fixer) => fixer.replaceTextRange(range, gap.replace(newline, newline.repeat(2))),
        });
      }
    }

    return {
      /** @param {any} program A Svelte program. */
      Program(program) {
        preserveWhitespace ||= hasPreserveWhitespaceOption(program);
        checkSiblings(program, program.body);
      },
      /** @param {any} container A template element or a single branch of a block. */
      [SIBLING_CONTAINERS](container) {
        checkSiblings(container, container.children);
      },
    };
  },
};
