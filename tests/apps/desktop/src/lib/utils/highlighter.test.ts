// Shiki syntax highlighting mapped to CSS classes (ADR-0079): detects shell
// languages from shell paths, produces CSS-class-only token spans with zero
// inline style properties, and falls back to null for unknown or oversized text.
import { describe, expect, it } from 'vitest';

import {
  highlightCode,
  isHighlightable,
  MAX_HIGHLIGHT_CHARS,
  MAX_HIGHLIGHT_LINE_CHARS,
  resolveLanguage,
  shellLanguage,
} from '../../../../../../apps/desktop/src/lib/utils/highlighter.js';

describe('highlighter', () => {
  it('detects shell languages from executable paths', () => {
    expect(shellLanguage('C:\\WINDOWS\\system32\\WindowsPowerShell\\v1.0\\powershell.exe')).toBe('powershell');
    expect(shellLanguage('C:\\Program Files\\PowerShell\\7\\pwsh.exe')).toBe('powershell');
    expect(shellLanguage('C:\\Windows\\System32\\cmd.exe')).toBe('bat');
    expect(shellLanguage('/usr/bin/fish')).toBe('fish');
    expect(shellLanguage('/bin/bash')).toBe('bash');
    expect(shellLanguage('/bin/sh')).toBe('bash');
    expect(shellLanguage('/bin/zsh')).toBe('bash');
    expect(shellLanguage(null)).toBe('bash');
  });

  it('resolves language names and aliases', () => {
    expect(resolveLanguage('ps1')).toBe('powershell');
    expect(resolveLanguage('sh')).toBe('bash');
    expect(resolveLanguage('CMD')).toBe('bat');
    expect(resolveLanguage('rs')).toBe('rust');
    expect(resolveLanguage('ts')).toBe('typescript');
    expect(resolveLanguage('unknown-lang')).toBeNull();
    expect(resolveLanguage('')).toBeNull();
  });

  it('highlights PowerShell, Bash, cmd, SQL, JSON and Rust with CSS classes and no inline styles', async () => {
    const samples: [string, string][] = [
      [
        'powershell',
        '"Shell: $($PSVersionTable.PSVersion)"; "Now: $(Get-Date -Format \'yyyy-MM-dd HH:mm:ss\')"; "User: $env:USERNAME"',
      ],
      ['bash', 'git status --short | grep -E "^M" && echo "$HOME"'],
      ['bat', 'echo %USERNAME% && dir /b C:\\'],
      ['sql', "SELECT id, name FROM parts WHERE stock > 10 AND name = 'bolt'"],
      ['json', '{\n  "command": "echo hello",\n  "timeout_seconds": 60\n}'],
      ['rust', 'pub fn add(a: i32, b: i32) -> i32 { a + b }'],
    ];

    for (const [language, code] of samples) {
      const lines = await highlightCode(code, language);
      expect(lines, language).not.toBeNull();
      const tokens = (lines ?? []).flatMap((line) => (typeof line === 'string' ? [] : line));
      expect(tokens.length, language).toBeGreaterThan(1);
      expect(
        tokens.some((token) => token.class?.startsWith('syntax-') ?? false),
        language,
      ).toBe(true);
      expect(
        tokens.every((token) => token.style === undefined),
        language,
      ).toBe(true);
      expect(
        (lines ?? [])
          .map((line) => (typeof line === 'string' ? line : line.map((token) => token.content).join('')))
          .join('\n'),
        language,
      ).toBe(code);
    }
  });

  it('rejects empty, oversized or over-long lines and unknown languages', async () => {
    expect(isHighlightable('')).toBe(false);
    expect(isHighlightable('a'.repeat(MAX_HIGHLIGHT_CHARS + 1))).toBe(false);
    expect(isHighlightable('a'.repeat(MAX_HIGHLIGHT_LINE_CHARS + 1))).toBe(false);
    expect(isHighlightable('echo ok')).toBe(true);

    expect(await highlightCode('a'.repeat(MAX_HIGHLIGHT_CHARS + 1), 'bash')).toBeNull();
    expect(await highlightCode('echo ok', 'does-not-exist')).toBeNull();
  });
});
