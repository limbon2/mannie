// @ts-check

const assignment = { selector: 'ExpressionStatement:has(> :matches(AssignmentExpression, UpdateExpression))' };
const awaited = { selector: 'ExpressionStatement:has(> AwaitExpression)' };
const callbackReturn = { selector: 'ReturnStatement:has(> :matches(ArrowFunctionExpression, FunctionExpression))' };

export const declarationPadding = [
  { blankLine: 'always', prev: 'import', next: '*' },
  { blankLine: 'any', prev: 'import', next: 'import' },
  { blankLine: 'always', prev: '*', next: 'function' },
  { blankLine: 'always', prev: 'function', next: '*' },
];

export const statementGroupPadding = [
  { blankLine: 'always', prev: ['const', 'let', 'var'], next: assignment },
  { blankLine: 'always', prev: assignment, next: ['try', awaited] },
  { blankLine: 'always', prev: awaited, next: assignment },
  { blankLine: 'always', prev: '*', next: callbackReturn },
];
