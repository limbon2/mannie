// @ts-check
import { svelteStatementGroup } from './svelte-script-order.mjs';

/**
 * @param {any} statement A Svelte script statement.
 * @returns {boolean}
 */
function declaresProps(statement) {
  if (statement.type !== 'VariableDeclaration') {
    return false;
  }

  return statement.declarations.some((/** @type {any} */ declaration) => {
    const initializer = declaration.init;
    if (initializer?.type !== 'CallExpression') {
      return false;
    }

    const callee = initializer.callee;
    if (callee.type === 'Identifier') {
      return callee.name === '$props';
    }

    const isMember = callee.type === 'MemberExpression';
    const isPropsObject = callee.object?.name === '$props';
    const isId = callee.property?.name === 'id';
    return isMember && isPropsObject && isId;
  });
}

/**
 * @param {any} statement A Svelte script statement.
 * @returns {boolean}
 */
function declaresFunction(statement) {
  if (statement.type === 'FunctionDeclaration') {
    return true;
  }

  const isExport = statement.type === 'ExportNamedDeclaration' || statement.type === 'ExportDefaultDeclaration';
  const exportsFunction = statement.declaration?.type === 'FunctionDeclaration';
  return isExport && exportsFunction;
}

/** @param {any} statement A Svelte script statement. */
function variableGroup(statement) {
  if (statement.type !== 'VariableDeclaration') {
    return undefined;
  }

  const group = svelteStatementGroup(statement);
  if (group === 'state') {
    return 'state';
  }
  if (group === 'other') {
    return statement.kind === 'const' ? 'constant' : 'mutable';
  }
  return undefined;
}

/**
 * @param {any} previous A Svelte script statement.
 * @param {any} current The following statement.
 * @returns {boolean}
 */
function needsPadding(previous, current) {
  const afterImports = previous.type === 'ImportDeclaration' && current.type !== 'ImportDeclaration';
  const afterProps = declaresProps(previous) && !declaresProps(current);
  const besideFunction = declaresFunction(previous) || declaresFunction(current);
  const previousGroup = variableGroup(previous);
  const currentGroup = variableGroup(current);
  const areVariableGroups = previousGroup !== undefined && currentGroup !== undefined;
  const changesVariableGroup = areVariableGroups && previousGroup !== currentGroup;
  return afterImports || afterProps || besideFunction || changesVariableGroup;
}

const ruleDescription = 'Separate imports, props, declaration groups and functions in Svelte scripts with blank lines';

/** @type {import('eslint').Rule.RuleModule} */
export const svelteScriptPadding = {
  meta: {
    type: 'layout',
    docs: { description: ruleDescription },
    fixable: 'whitespace',
    messages: { blankLine: 'Separate these groups with a blank line.' },
    schema: [],
  },
  create(context) {
    const { sourceCode } = context;
    const newline = sourceCode.text.match(/\r\n|\n|\r/u)?.[0] ?? '\n';

    /**
     * @param {any} previous A Svelte script statement.
     * @param {any} current The following statement.
     */
    function reportMissingPadding(previous, current) {
      let before = previous;
      let after = current;

      for (const comment of sourceCode.getCommentsBefore(current)) {
        // Keep end-of-line notes with the preceding statement and leave leading
        // comments, including next-line directives, beside the statement they describe.
        const startsOnLaterLine = comment.loc.start.line > before.loc.end.line;
        const appliesToNextLine = /\beslint-disable-next-line\b|@ts-(?:expect-error|ignore)\b/u.test(comment.value);
        if (startsOnLaterLine || appliesToNextLine) {
          after = comment;
          break;
        }

        before = comment;
      }

      const hasBlankLine = after.loc.start.line - before.loc.end.line > 1;
      if (hasBlankLine) {
        return;
      }

      context.report({
        node: current,
        messageId: 'blankLine',
        fix(fixer) {
          /** @type {[number, number]} */
          const range = [before.range[1], after.range[0]];
          const gap = sourceCode.text.slice(...range);
          if (gap.trim()) {
            return null;
          }

          const indentationLine = after.loc.start.line === before.loc.end.line ? before.loc.start : after.loc.start;
          const indentation = sourceCode.lines[indentationLine.line - 1].match(/^[\t ]*/u)?.[0] ?? '';
          return fixer.replaceTextRange(range, `${newline}${newline}${indentation}`);
        },
      });
    }

    return {
      /** @param {any} element A Svelte script element. */
      SvelteScriptElement(element) {
        for (let index = 1; index < element.body.length; index += 1) {
          const previous = element.body[index - 1];
          const current = element.body[index];
          if (needsPadding(previous, current)) {
            reportMissingPadding(previous, current);
          }
        }
      },
    };
  },
};
