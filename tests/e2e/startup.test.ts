import { waitForApp } from './support/page.ts';

describe('the WebDriver connection', () => {
  it('starts the app and reaches its main view', async () => {
    await waitForApp();
    await expect($('main')).toBeDisplayed();
  });
});
