const EXTENSION_LANGUAGES = {
  en: {directory: 'en', label: 'English'},
  zh: {directory: 'zh_CN', label: '简体中文'},
  'zh-Hant': {directory: 'zh_TW', label: '繁體中文'},
  ja: {directory: 'ja', label: '日本語'},
  ko: {directory: 'ko', label: '한국어'},
  es: {directory: 'es', label: 'Español'},
  'pt-BR': {directory: 'pt_BR', label: 'Português (Brasil)'},
  fr: {directory: 'fr', label: 'Français'},
  de: {directory: 'de', label: 'Deutsch'},
};
const extensionCatalogs = new Map();
const catalogLoads = new Map();

function extensionLocale(preference = 'auto', browserLanguage = 'en') {
  const requested = preference && preference !== 'auto' ? preference : browserLanguage;
  const tag = String(requested).toLowerCase().replaceAll('_', '-');
  const [language, ...subtags] = tag.split('-');
  if (language === 'zh') {
    if (subtags.includes('hans')) return 'zh';
    return subtags.some(part => ['hant', 'tw', 'hk', 'mo'].includes(part)) ? 'zh-Hant' : 'zh';
  }
  if (language === 'pt') return 'pt-BR';
  if (Object.hasOwn(EXTENSION_LANGUAGES, language)) return language;
  return preference && preference !== 'auto' ? extensionLocale('auto', browserLanguage) : 'en';
}

function installCatalog(locale, catalog) {
  if (catalog && Object.hasOwn(EXTENSION_LANGUAGES, locale)) extensionCatalogs.set(locale, catalog);
}

async function loadCatalog(locale) {
  if (extensionCatalogs.has(locale)) return extensionCatalogs.get(locale);
  const entry = EXTENSION_LANGUAGES[locale];
  if (!entry) return undefined;
  if (!catalogLoads.has(locale)) {
    catalogLoads.set(locale, (async () => {
      try {
        const response = await fetch(chrome.runtime.getURL(`_locales/${entry.directory}/messages.json`));
        if (!response.ok) return undefined;
        const catalog = await response.json();
        installCatalog(locale, catalog);
        return catalog;
      } catch { return undefined; }
      finally { catalogLoads.delete(locale); }
    })());
  }
  return catalogLoads.get(locale);
}

function extensionMessage(key, locale, ...values) {
  const message = extensionCatalogs.get(locale)?.[key]?.message || extensionCatalogs.get('en')?.[key]?.message;
  if (message) return message.replace(/\$(\d+)/g, (_, index) => String(values[Number(index) - 1] ?? ''));
  return (typeof chrome !== 'undefined' && chrome.i18n?.getMessage?.(key, values.map(String))) || key;
}

function localizeDocument(locale) {
  document.documentElement.lang = locale === 'zh' ? 'zh-CN' : locale;
  for (const node of document.querySelectorAll('[data-i18n]')) {
    node.textContent = extensionMessage(node.dataset.i18n, locale);
  }
  for (const [selector, attribute, key] of [
    ['[data-aria]', 'aria-label', 'aria'],
    ['[data-placeholder]', 'placeholder', 'placeholder'],
    ['[data-title]', 'title', 'title'],
  ]) {
    for (const node of document.querySelectorAll(selector)) {
      node.setAttribute(attribute, extensionMessage(node.dataset[key], locale));
    }
  }
}

if (typeof module !== 'undefined') {
  for (const [locale, entry] of Object.entries(EXTENSION_LANGUAGES)) {
    installCatalog(locale, require(`./_locales/${entry.directory}/messages.json`));
  }
  module.exports = {extensionLocale, extensionMessage, installCatalog};
}
