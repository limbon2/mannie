// Skills (docs/features/skills.md, acceptance): the Skills section lists this
// computer's skills on and a project's off, a switch turns a project's skill
// on from the keyboard, a reply's request holds the list and `read_skill`,
// whose read of a skill outside the workspace runs without asking, and "@"
// offers a skill with its token. The validation build reads no skills of its
// own; the tests name a folder for this computer.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  callMessage,
  COMPOSER,
  send,
  startNewConversation,
  waitForCompleteReply,
  waitForComposer,
} from '../support/chat.ts';
import { deleteProviderProfiles, setSkillsFolder } from '../support/harness.ts';
import { axeViolations, leaveSettings, openSettings, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const SECTION = 'skills.settings';
const COMPUTER_SKILL = '[data-skill-place="computer"] [data-skill="pdf-forms"] [data-skill-switch]';
const PROJECT_SKILL = '[data-skill-place="project"] [data-skill="docx"] [data-skill-switch]';
const OPTION = '[data-mention-list] [role="option"]';

/** Writes a skill into a place. */
function writeSkill(place: string, name: string, description: string): void {
  const folder = join(place, name);
  mkdirSync(folder, { recursive: true });
  writeFileSync(
    join(folder, 'SKILL.md'),
    `---\nname: ${name}\ndescription: ${description}\n---\n\n# ${name}\nTHE ${name} INSTRUCTIONS.\n`,
  );
}

/** The bodies of the requests for the tools scenario so far, the latest last. */
function toolsBodies(provider: ProviderServer): Record<string, unknown>[] {
  return provider.requests().flatMap(({ body }) => (body?.model === MODELS.tools ? [body] : []));
}

/** Whether a switch is on. */
async function isOn(selector: string): Promise<boolean> {
  return (await $(selector).getAttribute('aria-checked')) === 'true';
}

/** The labels of the "@" list's options, in order. */
async function optionLabels(): Promise<string[]> {
  return browser.execute(
    (selector: string) =>
      [...document.querySelectorAll(selector)].map(
        (option) => option.querySelector('[data-content]')?.textContent.trim() ?? '',
      ),
    OPTION,
  );
}

describe('skills', () => {
  let provider: ProviderServer;
  let home: string;
  let project: string;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    home = realpathSync(mkdtempSync(join(tmpdir(), 'mannie-skills-home-')));
    project = realpathSync(mkdtempSync(join(tmpdir(), 'mannie-skills-project-')));
    writeSkill(join(home, '.agents', 'skills'), 'pdf-forms', 'Fills PDF forms.');
    writeSkill(join(project, '.agents', 'skills'), 'docx', 'Writes Word documents.');
    await setSkillsFolder(join(home, '.agents', 'skills'));
  });

  after(async () => {
    await setSkillsFolder(null);
    await deleteProviderProfiles();
    await provider.close();
    rmSync(home, { force: true, recursive: true });
    rmSync(project, { force: true, recursive: true });
  });

  it("lists this computer's skills on and a project's off, and turns one on from the keyboard", async () => {
    await addProject(project);
    await openSettings(SECTION);
    await $(COMPUTER_SKILL).waitForDisplayed();
    await $(PROJECT_SKILL).waitForDisplayed();

    expect(await isOn(COMPUTER_SKILL)).toBe(true);
    expect(await isOn(PROJECT_SKILL)).toBe(false);
    expect(await axeViolations()).toEqual([]);

    await browser.execute((selector: string) => document.querySelector<HTMLElement>(selector)?.focus(), PROJECT_SKILL);
    await browser.keys(Key.Space);
    await browser.waitUntil(async () => isOn(PROJECT_SKILL), {
      timeout: 10_000,
      timeoutMsg: "the project's skill did not turn on",
    });
  });

  it('gives agents the list and read_skill, which reads a skill of this computer without asking', async () => {
    await leaveSettings();
    await startNewConversation();
    await waitForComposer();

    await send(callMessage('read_skill', { limit: null, name: 'pdf-forms', offset: null, path: null }));
    // The reply ends without the owner, so the read outside the workspace
    // asked nothing.
    await waitForCompleteReply();

    const bodies = toolsBodies(provider);
    const first = JSON.stringify(bodies.at(-2) ?? null);
    const last = JSON.stringify(bodies.at(-1) ?? null);
    expect(first).toContain('"read_skill"');
    expect(first).toContain('pdf-forms: Fills PDF forms.');
    expect(last).toContain('THE pdf-forms INSTRUCTIONS.');
  });

  it('offers a skill after "@" with a token of word characters', async () => {
    await startNewConversation();
    await waitForComposer();
    await $(COMPOSER).click();

    await browser.keys('@pdf');

    await browser.waitUntil(async () => (await optionLabels()).includes('pdf-forms'), {
      timeout: 10_000,
      timeoutMsg: 'the list did not offer the skill',
    });
    await browser.keys(Key.Enter);
    await browser.waitUntil(
      async () =>
        browser.execute(
          (box: string) =>
            document.querySelector<HTMLTextAreaElement>(box)?.value.includes('@skill_pdf_forms') ?? false,
          COMPOSER,
        ),
      { timeout: 10_000, timeoutMsg: 'the token did not enter the message box' },
    );
  });
});
