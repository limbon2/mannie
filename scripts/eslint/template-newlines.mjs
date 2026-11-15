// @ts-check

const BLOCK_MARKER = /^(?:[#/](?:if|each|await|key|snippet)|:(?:else|then|catch))$/u;

const STRUCTURAL_CHILD = /^(?:SvelteElement|Svelte(?:If|Each|Await|Key|Snippet)Block|SvelteHTMLComment)$/u;

const INLINE_ELEMENTS = new Set([
  'a',
  'abbr',
  'b',
  'button',
  'cite',
  'code',
  'del',
  'dfn',
  'em',
  'i',
  'ins',
  'kbd',
  'label',
  'mark',
  'q',
  's',
  'samp',
  'small',
  'span',
  'strong',
  'sub',
  'sup',
  'text',
  'time',
  'u',
  'var',
]);

const PREFORMATTED_ELEMENTS = new Set(['pre', 'textarea']);

const PRESERVED_WHITESPACE =
  /\bwhitespace-(?:pre(?:-line|-wrap)?|break-spaces)\b|\b(?:white-space:\s*(?:pre|break-spaces)|style:white-space\b)/u;

/** An identifier in the name position of a Svelte element refers to a component. @param {any} node */
export function elementName(node) {
  const isElement = node.type === 'SvelteElement';
  const isHtmlName = node.name?.type === 'SvelteName';
  return isElement && isHtmlName ? node.name.name : '';
}

/** @param {any} node A template child. */
export function isWhitespace(node) {
  if (node.type !== 'SvelteText') {
    return false;
  }

  return node.value.trim() === '';
}

/** @param {any} node A template child. */
function isInlineContent(node) {
  const isText = node.type === 'SvelteText' && !isWhitespace(node);
  const isExpression = /^(?:SvelteMustacheTag|SvelteRawMustacheTag|SvelteRenderTag)$/u.test(node.type);
  return isText || isExpression || INLINE_ELEMENTS.has(elementName(node));
}

/** @param {any} left A template node. @param {any} right Another template node. */
function sharesSourceLines(left, right) {
  const leftEndsAfterRightStarts = left.loc.end.line >= right.loc.start.line;
  const rightEndsAfterLeftStarts = right.loc.end.line >= left.loc.start.line;
  return leftEndsAfterRightStarts && rightEndsAfterLeftStarts;
}

/**
 * Source newlines can become visible whitespace in prose or preformatted content.
 * @param {any} node A template node.
 * @param {import('eslint').SourceCode} sourceCode
 */
export function hasWhitespaceSensitiveContext(node, sourceCode) {
  let inContainingElement = false;

  for (let ancestor = node; ancestor; ancestor = ancestor.parent) {
    const name = elementName(ancestor);
    const isPreformatted = PREFORMATTED_ELEMENTS.has(name);
    const keepsWhitespace = ancestor.startTag?.attributes.some((/** @type {any} */ attribute) =>
      PRESERVED_WHITESPACE.test(sourceCode.getText(attribute)),
    );
    if (isPreformatted || keepsWhitespace) {
      return true;
    }

    if (inContainingElement) {
      continue;
    }

    const isProseElement = name === 'p' || INLINE_ELEMENTS.has(name);
    const atRoot = ancestor.type === 'Program';
    const children = atRoot ? ancestor.body : ancestor.children;
    const hasInlineChildren = children?.some((/** @type {any} */ child) => {
      const mayShareProse = !atRoot || sharesSourceLines(child, node);
      return mayShareProse && isInlineContent(child);
    });
    if (isProseElement || hasInlineChildren) {
      return true;
    }

    inContainingElement = ancestor.type === 'SvelteElement';
  }

  return false;
}

/**
 * A marker's expression can contain object literals and destructuring braces.
 * @param {any[]} tokens
 * @param {number} openingIndex
 */
function findMarkerEnd(tokens, openingIndex) {
  let depth = 0;

  for (let index = openingIndex; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token.type !== 'Punctuator') {
      continue;
    }

    if (token.value === '{') {
      depth += 1;
    } else if (token.value === '}') {
      depth -= 1;
      if (depth === 0) {
        return token;
      }
    }
  }

  return undefined;
}

/** @type {import('eslint').Rule.RuleModule} */
export const templateNewlines = {
  meta: {
    type: 'layout',
    docs: { description: 'Put structural Svelte template markup on separate lines' },
    messages: {
      block: 'Put the {{keyword}} block marker on its own line.',
      nested: 'Put nested elements and blocks on separate lines from their enclosing tags and siblings.',
    },
    schema: [],
  },
  create(context) {
    const { sourceCode } = context;

    return {
      /** @param {any} program A Svelte program. */
      Program(program) {
        const { tokens } = program;

        for (let index = 1; index < tokens.length; index += 1) {
          const marker = tokens[index];
          const isBlockMarker = marker.type === 'MustacheKeyword' && BLOCK_MARKER.test(marker.value);
          if (!isBlockMarker) {
            continue;
          }

          const opening = tokens[index - 1];
          const opensMarker = opening.type === 'Punctuator' && opening.value === '{';
          if (!opensMarker) {
            continue;
          }

          const closing = findMarkerEnd(tokens, index - 1);
          if (!closing) {
            continue;
          }

          const before = sourceCode.lines[opening.loc.start.line - 1].slice(0, opening.loc.start.column);
          const after = sourceCode.lines[closing.loc.end.line - 1].slice(closing.loc.end.column);
          const sharesLine = before.trim() !== '' || after.trim() !== '';
          const owner = sourceCode.getNodeByRangeIndex(marker.range[0]);
          if (sharesLine && !hasWhitespaceSensitiveContext(owner, sourceCode)) {
            context.report({ loc: marker.loc, messageId: 'block', data: { keyword: marker.value } });
          }
        }
      },
      /** @param {any} node A Svelte element. */
      SvelteElement(node) {
        const children = node.children.filter((/** @type {any} */ child) => !isWhitespace(child));
        const hasChildren = children.length > 0;
        const hasOnlyStructuralChildren = children.every((/** @type {any} */ child) =>
          STRUCTURAL_CHILD.test(child.type),
        );
        if (!node.endTag || !hasChildren || !hasOnlyStructuralChildren) {
          return;
        }

        if (hasWhitespaceSensitiveContext(node, sourceCode)) {
          return;
        }

        const parts = [node.startTag, ...children, node.endTag];
        const crowded = parts.find((part, index) => {
          const previous = parts[index - 1];
          return previous?.loc.end.line === part.loc.start.line;
        });
        if (crowded) {
          const token = sourceCode.getFirstToken(crowded);
          context.report({ loc: token?.loc ?? crowded.loc, messageId: 'nested' });
        }
      },
    };
  },
};
