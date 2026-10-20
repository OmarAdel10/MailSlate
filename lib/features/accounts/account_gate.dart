import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/tokens.dart';
import 'package:mailslate/features/accounts/account_gate_bloc.dart';
import 'package:mailslate/features/accounts/wizard/account_wizard_screen.dart';
import 'package:mailslate/features/shell/shell_screen.dart';

/// Startup gate between the first-run wizard and the shell
/// (USER_FLOW §3): accounts exist → shell, zero accounts → wizard.
///
/// A failed listing still shows the shell: the shell's paused banner
/// already reports daemon outages (M1 resilience), and hiding the app
/// behind an error screen would strand a user whose daemon is merely
/// restarting.
class AccountGate extends StatelessWidget {
  const AccountGate({super.key, this.openThreadId});

  /// Conversation to reveal when deep-linking lands (M3); forwarded to
  /// the shell when it is shown.
  final String? openThreadId;

  @override
  Widget build(BuildContext context) {
    return BlocProvider(
      create: (_) =>
          AccountGateBloc(getIt<FfiClient>())
            ..add(const AccountGateCheckRequested()),
      child: _AccountGateView(openThreadId: openThreadId),
    );
  }
}

class _AccountGateView extends StatelessWidget {
  const _AccountGateView({this.openThreadId});

  final String? openThreadId;

  @override
  Widget build(BuildContext context) {
    return BlocBuilder<AccountGateBloc, AccountGateState>(
      builder: (context, state) {
        if (state is AccountGateNeedsAccount) {
          return AccountWizardScreen(
            firstAccount: true,
            onDone: () => context.read<AccountGateBloc>().add(
              const AccountGateCheckRequested(),
            ),
          );
        }
        if (state is AccountGateChecking) {
          return const _CheckingView();
        }
        // Ready and failure both land on the shell: the banner reports
        // daemon outages, and a ready gate needs no extra decoration.
        return ShellScreen(openThreadId: openThreadId);
      },
    );
  }
}

class _CheckingView extends StatelessWidget {
  const _CheckingView();

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Scaffold(
      key: const Key('account-gate-checking'),
      backgroundColor: tokens.canvas,
      body: const Center(child: CircularProgressIndicator()),
    );
  }
}
