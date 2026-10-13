// Link validation for the documentation: every relative link and heading
// anchor must resolve. remark-validate-links works offline, so external links
// are not checked here.
import remarkValidateLinks from 'remark-validate-links';

export default {
  plugins: [[remarkValidateLinks, { repository: false }]],
};
