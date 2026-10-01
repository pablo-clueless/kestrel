/** Matches the engine (`auth::password::check_policy`): length only, any characters, so
 * passphrases and non-ASCII work (NIST 800-63B). The engine is what enforces it. */
export const PASSWORD_MIN = 8;
export const PASSWORD_MAX = 128;

export const PASSWORD_MESSAGE = `Use ${PASSWORD_MIN} to ${PASSWORD_MAX} characters. A few words make a strong password.`;
