class DictionaryError extends Error {
  constructor(status, code, message) {
    super(message);
    this.name = 'DictionaryError';
    this.status = status;
    this.code = code;
  }
}
const notFound = () => new DictionaryError(404, 'WORD_NOT_FOUND', 'word not found');
const invalidPage = (reason) => new DictionaryError(502, 'INVALID_CAMBRIDGE_PAGE', `Cambridge page is incomplete or unsupported: ${reason}`);
module.exports = { DictionaryError, notFound, invalidPage };
