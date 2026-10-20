import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/ffi/events.dart';
import 'package:mailslate/core/system/url_opener.dart';
import 'package:mailslate/core/theme/tokens.dart';

import 'account_wizard_bloc.dart';
import 'provider_presets.dart';

/// The account onboarding wizard (USER_FLOW §3).
///
/// Shown on first run before the shell, and again for every additional
/// account (`firstAccount: false` skips the welcome step). [onDone] fires
/// once the daemon reports the account row.
class AccountWizardScreen extends StatelessWidget {
  const AccountWizardScreen({super.key, this.firstAccount = true, this.onDone});

  /// Whether this is the first account (show the welcome step).
  final bool firstAccount;

  /// Called once the account exists on the daemon.
  final VoidCallback? onDone;

  @override
  Widget build(BuildContext context) {
    final client = getIt<FfiClient>();
    return BlocProvider(
      create: (_) => AccountWizardBloc(
        client,
        getIt<UrlOpener>(),
        events: appEventStream(client),
        firstAccount: firstAccount,
      ),
      child: _WizardView(onDone: onDone),
    );
  }
}

class _WizardView extends StatelessWidget {
  const _WizardView({required this.onDone});

  final VoidCallback? onDone;

  @override
  Widget build(BuildContext context) {
    return BlocListener<AccountWizardBloc, AccountWizardState>(
      listenWhen: (previous, current) => current is AccountConnected,
      listener: (context, state) => onDone?.call(),
      child: BlocBuilder<AccountWizardBloc, AccountWizardState>(
        builder: (context, state) {
          return Scaffold(
            backgroundColor: context.tokens.canvas,
            body: Center(
              child: SingleChildScrollView(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 480),
                  child: switch (state) {
                    WizardWelcome() => const _WelcomeView(),
                    ChooseProvider(:final note) => _ProviderPickView(
                      note: note,
                    ),
                    WaitingForOauth() => const _WaitingOauthView(),
                    OauthTimedOut() => const _TimeoutView(),
                    EnterImapDetails() => _ImapFormView(
                      key: const Key('wizard-imap-form'),
                      details: state,
                    ),
                    TestingImapConnection() => const _ImapFormView(
                      key: Key('wizard-imap-form'),
                      busy: true,
                    ),
                    AddingAccount() => const _ImapFormView(
                      key: Key('wizard-imap-form'),
                      busy: true,
                    ),
                    AccountConnected() => const _ConnectedView(),
                  },
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// Step 1: welcome headline and the two entry buttons.
class _WelcomeView extends StatelessWidget {
  const _WelcomeView();

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Padding(
      key: const Key('wizard-welcome'),
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Welcome to MailSlate',
            style: textTheme.displaySmall?.copyWith(color: tokens.ink),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 8),
          Text(
            'Fast, local-first email for your desktop.',
            style: textTheme.bodySmall?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 24),
          FilledButton(
            key: const Key('wizard-connect-account'),
            onPressed: () => context.read<AccountWizardBloc>().add(
              const ConnectAccountRequested(),
            ),
            child: const Text('Connect account'),
          ),
          const SizedBox(height: 8),
          // No data exists before the first account (USER_FLOW §3 step 1).
          const OutlinedButton(
            key: Key('wizard-explore-offline'),
            onPressed: null,
            child: Text('Explore offline'),
          ),
        ],
      ),
    );
  }
}

/// Step 2: the three provider cards.
class _ProviderPickView extends StatelessWidget {
  const _ProviderPickView({this.note});

  /// Inline note above the cards (denied consent, failed start).
  final String? note;

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Padding(
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Connect your mail',
            style: textTheme.displaySmall?.copyWith(color: tokens.ink),
            textAlign: TextAlign.center,
          ),
          if (note != null) ...[
            const SizedBox(height: 8),
            Text(
              note!,
              key: const Key('wizard-pick-note'),
              style: textTheme.bodySmall?.copyWith(color: tokens.error),
              textAlign: TextAlign.center,
            ),
          ],
          const SizedBox(height: 24),
          _ProviderCard(
            key: const Key('wizard-card-gmail'),
            icon: Symbols.mail,
            label: 'Gmail',
            hint: 'Sign in with Google in your browser',
            onTap: () => context.read<AccountWizardBloc>().add(
              const ProviderPicked(provider: WizardProvider.gmail),
            ),
          ),
          const SizedBox(height: 12),
          _ProviderCard(
            key: const Key('wizard-card-outlook'),
            icon: Symbols.all_inbox,
            label: 'Outlook',
            hint: 'Sign in with Microsoft in your browser',
            onTap: () => context.read<AccountWizardBloc>().add(
              const ProviderPicked(provider: WizardProvider.outlook),
            ),
          ),
          const SizedBox(height: 12),
          _ProviderCard(
            key: const Key('wizard-card-other'),
            icon: Symbols.dns,
            label: 'Other IMAP',
            hint: 'Host, port, and password',
            onTap: () => context.read<AccountWizardBloc>().add(
              const ProviderPicked(provider: WizardProvider.otherImap),
            ),
          ),
        ],
      ),
    );
  }
}

class _ProviderCard extends StatelessWidget {
  const _ProviderCard({
    super.key,
    required this.icon,
    required this.label,
    required this.hint,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final String hint;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Material(
      color: tokens.surfaceCard,
      borderRadius: BorderRadius.circular(12),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        hoverColor: tokens.surfaceStrong,
        child: Container(
          height: 72,
          padding: const EdgeInsets.symmetric(horizontal: 16),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: tokens.hairline),
          ),
          child: Row(
            children: [
              Icon(icon, size: 20, color: tokens.ink),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      label,
                      style: textTheme.titleSmall?.copyWith(color: tokens.ink),
                    ),
                    Text(
                      hint,
                      style: textTheme.bodySmall?.copyWith(color: tokens.muted),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Step 3: waiting for the browser round-trip.
class _WaitingOauthView extends StatelessWidget {
  const _WaitingOauthView();

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Padding(
      key: const Key('wizard-oauth-waiting'),
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Complete sign-in in your browser…',
            style: textTheme.titleMedium?.copyWith(color: tokens.ink),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 8),
          Text(
            'MailSlate is waiting for the sign-in to finish.',
            style: textTheme.bodySmall?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 16),
          TextButton(
            key: const Key('wizard-reopen-browser'),
            onPressed: () => context.read<AccountWizardBloc>().add(
              const ReopenBrowserRequested(),
            ),
            child: const Text('Reopen browser'),
          ),
        ],
      ),
    );
  }
}

/// The loopback listener timed out: offer `Try again`.
class _TimeoutView extends StatelessWidget {
  const _TimeoutView();

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Padding(
      key: const Key('wizard-oauth-timeout'),
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Sign-in timed out',
            style: textTheme.titleMedium?.copyWith(color: tokens.ink),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 8),
          Text(
            'The browser sign-in did not finish in two minutes.',
            style: textTheme.bodySmall?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const Key('wizard-try-again'),
            onPressed: () => context.read<AccountWizardBloc>().add(
              const OauthRetryRequested(),
            ),
            child: const Text('Try again'),
          ),
        ],
      ),
    );
  }
}

/// Step 4–5: the Other IMAP form and its connection-test row.
class _ImapFormView extends StatefulWidget {
  const _ImapFormView({super.key, this.details, this.busy = false});

  /// The current wizard state, when the form is idle.
  final EnterImapDetails? details;

  /// A test or save is in flight; buttons are disabled.
  final bool busy;

  @override
  State<_ImapFormView> createState() => _ImapFormViewState();
}

class _ImapFormViewState extends State<_ImapFormView> {
  final _displayName = TextEditingController();
  final _email = TextEditingController();
  final _password = TextEditingController();
  final _imapHost = TextEditingController();
  final _imapPort = TextEditingController();
  final _smtpHost = TextEditingController();
  final _smtpPort = TextEditingController();

  String _imapSecurity = 'tls';
  String _smtpSecurity = 'tls';
  final Map<String, String> _errors = {};

  static const _securityChoices = {
    'SSL': 'tls',
    'STARTTLS': 'starttls',
    'Plaintext': 'plaintext',
  };

  @override
  void dispose() {
    _displayName.dispose();
    _email.dispose();
    _password.dispose();
    _imapHost.dispose();
    _imapPort.dispose();
    _smtpHost.dispose();
    _smtpPort.dispose();
    super.dispose();
  }

  void _applyPreset(ImapPreset preset) {
    setState(() {
      _imapHost.text = preset.imapHost;
      _imapPort.text = preset.imapPort.toString();
      _imapSecurity = preset.imapSecurity;
      _smtpHost.text = preset.smtpHost;
      _smtpPort.text = preset.smtpPort.toString();
      _smtpSecurity = preset.smtpSecurity;
      _errors.clear();
    });
  }

  ImapDraft _draft() => ImapDraft(
    displayName: _displayName.text.trim().isEmpty
        ? null
        : _displayName.text.trim(),
    email: _email.text.trim(),
    imapHost: _imapHost.text.trim(),
    imapPort: int.tryParse(_imapPort.text.trim()) ?? 0,
    imapSecurity: _imapSecurity,
    smtpHost: _smtpHost.text.trim(),
    smtpPort: int.tryParse(_smtpPort.text.trim()) ?? 0,
    smtpSecurity: _smtpSecurity,
  );

  String get _passwordValue => _password.text;

  bool _validate() {
    final errors = <String, String>{};
    bool missing(TextEditingController c) => c.text.trim().isEmpty;
    for (final entry in {
      'email': _email,
      'password': _password,
      'imapHost': _imapHost,
      'imapPort': _imapPort,
      'smtpHost': _smtpHost,
      'smtpPort': _smtpPort,
    }.entries) {
      if (missing(entry.value)) {
        errors[entry.key] = 'Required';
      }
    }
    if (errors['imapPort'] == null &&
        int.tryParse(_imapPort.text.trim()) == null) {
      errors['imapPort'] = 'Enter a number';
    }
    if (errors['smtpPort'] == null &&
        int.tryParse(_smtpPort.text.trim()) == null) {
      errors['smtpPort'] = 'Enter a number';
    }
    setState(
      () => _errors
        ..clear()
        ..addAll(errors),
    );
    return errors.isEmpty;
  }

  void _testConnection() {
    if (!_validate()) {
      return;
    }
    context.read<AccountWizardBloc>().add(
      ImapConnectionTestRequested(draft: _draft(), password: _passwordValue),
    );
  }

  void _saveAccount() {
    context.read<AccountWizardBloc>().add(
      ImapAccountSaveRequested(draft: _draft(), password: _passwordValue),
    );
  }

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    final lastTest = widget.details?.lastTest;
    final note = widget.details?.note;
    final bothOk = lastTest != null && lastTest.imap.ok && lastTest.smtp.ok;
    return Padding(
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Connect an IMAP account',
            style: textTheme.displaySmall?.copyWith(color: tokens.ink),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 16),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              for (final preset in imapPresets)
                OutlinedButton(
                  key: Key('wizard-preset-${preset.label}'),
                  onPressed: widget.busy ? null : () => _applyPreset(preset),
                  child: Text(preset.label),
                ),
              const OutlinedButton(
                key: Key('wizard-preset-Custom'),
                onPressed: null,
                child: Text('Custom'),
              ),
            ],
          ),
          const SizedBox(height: 16),
          _Field(
            key: const Key('wizard-field-display-name'),
            controller: _displayName,
            label: 'Display name (optional)',
          ),
          const SizedBox(height: 12),
          _Field(
            key: const Key('wizard-field-email'),
            controller: _email,
            label: 'Email',
            errorText: _errors['email'],
          ),
          const SizedBox(height: 12),
          _Field(
            key: const Key('wizard-field-password'),
            controller: _password,
            label: 'Password',
            obscure: true,
            errorText: _errors['password'],
          ),
          const SizedBox(height: 16),
          Row(
            children: [
              Expanded(
                child: _Field(
                  key: const Key('wizard-field-imap-host'),
                  controller: _imapHost,
                  label: 'IMAP host',
                  errorText: _errors['imapHost'],
                ),
              ),
              const SizedBox(width: 12),
              SizedBox(
                width: 96,
                child: _Field(
                  key: const Key('wizard-field-imap-port'),
                  controller: _imapPort,
                  label: 'Port',
                  number: true,
                  errorText: _errors['imapPort'],
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          _SecurityField(
            key: const Key('wizard-field-imap-security'),
            label: 'IMAP security',
            value: _imapSecurity,
            onChanged: widget.busy
                ? null
                : (value) => setState(() => _imapSecurity = value),
          ),
          ExpansionTile(
            key: const Key('wizard-smtp-section'),
            title: Text(
              'SMTP overrides (optional)',
              style: textTheme.bodySmall?.copyWith(color: tokens.body),
            ),
            children: [
              const SizedBox(height: 4),
              Row(
                children: [
                  Expanded(
                    child: _Field(
                      key: const Key('wizard-field-smtp-host'),
                      controller: _smtpHost,
                      label: 'SMTP host',
                      errorText: _errors['smtpHost'],
                    ),
                  ),
                  const SizedBox(width: 12),
                  SizedBox(
                    width: 96,
                    child: _Field(
                      key: const Key('wizard-field-smtp-port'),
                      controller: _smtpPort,
                      label: 'Port',
                      number: true,
                      errorText: _errors['smtpPort'],
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              _SecurityField(
                key: const Key('wizard-field-smtp-security'),
                label: 'SMTP security',
                value: _smtpSecurity,
                onChanged: widget.busy
                    ? null
                    : (value) => setState(() => _smtpSecurity = value),
              ),
            ],
          ),
          if (note != null) ...[
            const SizedBox(height: 4),
            Text(
              note,
              style: textTheme.bodySmall?.copyWith(color: tokens.error),
            ),
          ],
          if (lastTest != null) ...[
            const SizedBox(height: 16),
            _StepRow(
              key: const Key('wizard-imap-step'),
              label: 'IMAP',
              step: lastTest.imap,
            ),
            const SizedBox(height: 4),
            _StepRow(
              key: const Key('wizard-smtp-step'),
              label: 'SMTP',
              step: lastTest.smtp,
            ),
          ],
          const SizedBox(height: 16),
          OutlinedButton(
            key: const Key('wizard-test-connection'),
            onPressed: widget.busy ? null : _testConnection,
            child: const Text('Test connection'),
          ),
          const SizedBox(height: 8),
          FilledButton(
            key: const Key('wizard-add-account'),
            onPressed: !widget.busy && bothOk ? _saveAccount : null,
            child: const Text('Add account'),
          ),
        ],
      ),
    );
  }
}

class _Field extends StatelessWidget {
  const _Field({
    super.key,
    required this.controller,
    required this.label,
    this.obscure = false,
    this.number = false,
    this.errorText,
  });

  final TextEditingController controller;
  final String label;
  final bool obscure;
  final bool number;
  final String? errorText;

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: controller,
      obscureText: obscure,
      keyboardType: number ? TextInputType.number : null,
      decoration: InputDecoration(
        labelText: label,
        errorText: errorText,
        isDense: true,
      ),
    );
  }
}

class _SecurityField extends StatelessWidget {
  const _SecurityField({
    super.key,
    required this.label,
    required this.value,
    this.onChanged,
  });

  final String label;
  final String value;
  final ValueChanged<String>? onChanged;

  @override
  Widget build(BuildContext context) {
    final handler = onChanged;
    return DropdownButtonFormField<String>(
      initialValue: value,
      decoration: InputDecoration(labelText: label, isDense: true),
      items: [
        for (final entry in _ImapFormViewState._securityChoices.entries)
          DropdownMenuItem(value: entry.value, child: Text(entry.key)),
      ],
      onChanged: handler == null
          ? null
          : (selection) {
              if (selection != null) {
                handler(selection);
              }
            },
    );
  }
}

/// One step of the `IMAP ✓ / SMTP ✓` progress row (USER_FLOW §3 step 5).
class _StepRow extends StatelessWidget {
  const _StepRow({super.key, required this.label, required this.step});

  final String label;
  final ConnectionStep step;

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Row(
      children: [
        if (step.ok)
          Icon(Symbols.check, size: 16, color: tokens.success)
        else
          Icon(Symbols.close, size: 16, color: tokens.error),
        const SizedBox(width: 8),
        Text(
          label,
          style: textTheme.bodySmall?.copyWith(
            color: step.ok ? tokens.body : tokens.error,
            fontWeight: FontWeight.w600,
          ),
        ),
        if (!step.ok && step.error != null) ...[
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              step.error!,
              style: textTheme.bodySmall?.copyWith(color: tokens.error),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ],
    );
  }
}

/// Step 6: the account exists and initial sync can start.
class _ConnectedView extends StatelessWidget {
  const _ConnectedView();

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final tokens = context.tokens;
    return Padding(
      key: const Key('wizard-connected'),
      padding: const EdgeInsets.all(32),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Symbols.check_circle, size: 48, color: tokens.success),
          const SizedBox(height: 16),
          Text(
            'Account connected',
            style: textTheme.titleMedium?.copyWith(color: tokens.ink),
          ),
          const SizedBox(height: 8),
          Text(
            'Your inbox will start filling in shortly.',
            style: textTheme.bodySmall?.copyWith(color: tokens.muted),
            textAlign: TextAlign.center,
          ),
        ],
      ),
    );
  }
}
