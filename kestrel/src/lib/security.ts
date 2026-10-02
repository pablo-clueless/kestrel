/**
 * The rules a new password must meet, one per line of the checklist. They mirror
 * `PASSWORD_REGEX` (`@/config/string`) so a fully ticked checklist — and so a
 * "Strong" badge — means the form will accept the password.
 */
export const PASSWORD_RULES = {
  length: (password: string) => password.length >= 12 && password.length <= 128,
  lowercase: (password: string) => /[a-z]/.test(password),
  uppercase: (password: string) => /[A-Z]/.test(password),
  number: (password: string) => /[0-9]/.test(password),
  special: (password: string) => /[^A-Za-z0-9]/.test(password),
} as const;

export type PasswordRule = keyof typeof PASSWORD_RULES;

export type PasswordStrength = "weak" | "fair" | "strong";

export const checkPasswordRules = (password: string): Record<PasswordRule, boolean> => ({
  length: PASSWORD_RULES.length(password),
  lowercase: PASSWORD_RULES.lowercase(password),
  uppercase: PASSWORD_RULES.uppercase(password),
  number: PASSWORD_RULES.number(password),
  special: PASSWORD_RULES.special(password),
});

/** "Strong" only when every rule is met — anything less is a password the form refuses. */
export const getPasswordStrength = (password: string): PasswordStrength | null => {
  if (!password) return null;
  const met = Object.values(checkPasswordRules(password)).filter(Boolean).length;
  if (met === Object.keys(PASSWORD_RULES).length) return "strong";
  if (met >= 3) return "fair";
  return "weak";
};
