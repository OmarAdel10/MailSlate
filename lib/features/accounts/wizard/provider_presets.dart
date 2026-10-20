/// Host presets for the Other IMAP wizard path (USER_FLOW §3 step 4).
///
/// Security values are the daemon wire strings (`tls`, `starttls`,
/// `plaintext`); the form labels map them to `SSL` / `STARTTLS`.
class ImapPreset {
  const ImapPreset({
    required this.label,
    required this.imapHost,
    required this.imapPort,
    required this.imapSecurity,
    required this.smtpHost,
    required this.smtpPort,
    required this.smtpSecurity,
  });

  /// Card label shown in the form.
  final String label;

  /// IMAP hostname.
  final String imapHost;

  /// IMAP port.
  final int imapPort;

  /// IMAP security (wire string).
  final String imapSecurity;

  /// SMTP hostname.
  final String smtpHost;

  /// SMTP port.
  final int smtpPort;

  /// SMTP security (wire string).
  final String smtpSecurity;
}

/// Yahoo Mail.
const yahooImapPreset = ImapPreset(
  label: 'Yahoo',
  imapHost: 'imap.mail.yahoo.com',
  imapPort: 993,
  imapSecurity: 'tls',
  smtpHost: 'smtp.mail.yahoo.com',
  smtpPort: 465,
  smtpSecurity: 'tls',
);

/// Fastmail.
const fastmailImapPreset = ImapPreset(
  label: 'Fastmail',
  imapHost: 'imap.fastmail.com',
  imapPort: 993,
  imapSecurity: 'tls',
  smtpHost: 'smtp.fastmail.com',
  smtpPort: 465,
  smtpSecurity: 'tls',
);

/// iCloud Mail.
const icloudImapPreset = ImapPreset(
  label: 'iCloud',
  imapHost: 'imap.mail.me.com',
  imapPort: 993,
  imapSecurity: 'tls',
  smtpHost: 'smtp.mail.me.com',
  smtpPort: 587,
  smtpSecurity: 'starttls',
);

/// Every built-in preset, in display order.
const imapPresets = <ImapPreset>[
  yahooImapPreset,
  fastmailImapPreset,
  icloudImapPreset,
];
