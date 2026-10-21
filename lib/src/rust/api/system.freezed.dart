// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'system.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$AppEvent {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'AppEvent()';
}


}

/// @nodoc
class $AppEventCopyWith<$Res>  {
$AppEventCopyWith(AppEvent _, $Res Function(AppEvent) __);
}


/// Adds pattern-matching-related methods to [AppEvent].
extension AppEventPatterns on AppEvent {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( AppEvent_AccountStatus value)?  accountStatus,TResult Function( AppEvent_SyncProgress value)?  syncProgress,TResult Function( AppEvent_OauthUrlReady value)?  oauthUrlReady,TResult Function( AppEvent_OauthCompleted value)?  oauthCompleted,TResult Function( AppEvent_OauthFailed value)?  oauthFailed,TResult Function( AppEvent_OauthReconnected value)?  oauthReconnected,TResult Function( AppEvent_Other value)?  other,required TResult orElse(),}){
final _that = this;
switch (_that) {
case AppEvent_AccountStatus() when accountStatus != null:
return accountStatus(_that);case AppEvent_SyncProgress() when syncProgress != null:
return syncProgress(_that);case AppEvent_OauthUrlReady() when oauthUrlReady != null:
return oauthUrlReady(_that);case AppEvent_OauthCompleted() when oauthCompleted != null:
return oauthCompleted(_that);case AppEvent_OauthFailed() when oauthFailed != null:
return oauthFailed(_that);case AppEvent_OauthReconnected() when oauthReconnected != null:
return oauthReconnected(_that);case AppEvent_Other() when other != null:
return other(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( AppEvent_AccountStatus value)  accountStatus,required TResult Function( AppEvent_SyncProgress value)  syncProgress,required TResult Function( AppEvent_OauthUrlReady value)  oauthUrlReady,required TResult Function( AppEvent_OauthCompleted value)  oauthCompleted,required TResult Function( AppEvent_OauthFailed value)  oauthFailed,required TResult Function( AppEvent_OauthReconnected value)  oauthReconnected,required TResult Function( AppEvent_Other value)  other,}){
final _that = this;
switch (_that) {
case AppEvent_AccountStatus():
return accountStatus(_that);case AppEvent_SyncProgress():
return syncProgress(_that);case AppEvent_OauthUrlReady():
return oauthUrlReady(_that);case AppEvent_OauthCompleted():
return oauthCompleted(_that);case AppEvent_OauthFailed():
return oauthFailed(_that);case AppEvent_OauthReconnected():
return oauthReconnected(_that);case AppEvent_Other():
return other(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( AppEvent_AccountStatus value)?  accountStatus,TResult? Function( AppEvent_SyncProgress value)?  syncProgress,TResult? Function( AppEvent_OauthUrlReady value)?  oauthUrlReady,TResult? Function( AppEvent_OauthCompleted value)?  oauthCompleted,TResult? Function( AppEvent_OauthFailed value)?  oauthFailed,TResult? Function( AppEvent_OauthReconnected value)?  oauthReconnected,TResult? Function( AppEvent_Other value)?  other,}){
final _that = this;
switch (_that) {
case AppEvent_AccountStatus() when accountStatus != null:
return accountStatus(_that);case AppEvent_SyncProgress() when syncProgress != null:
return syncProgress(_that);case AppEvent_OauthUrlReady() when oauthUrlReady != null:
return oauthUrlReady(_that);case AppEvent_OauthCompleted() when oauthCompleted != null:
return oauthCompleted(_that);case AppEvent_OauthFailed() when oauthFailed != null:
return oauthFailed(_that);case AppEvent_OauthReconnected() when oauthReconnected != null:
return oauthReconnected(_that);case AppEvent_Other() when other != null:
return other(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( PlatformInt64 account,  String state)?  accountStatus,TResult Function( PlatformInt64 account,  String mailbox,  String phase,  int done,  int total)?  syncProgress,TResult Function( String url)?  oauthUrlReady,TResult Function( String email)?  oauthCompleted,TResult Function( String reason)?  oauthFailed,TResult Function( PlatformInt64 account,  String email)?  oauthReconnected,TResult Function( String name)?  other,required TResult orElse(),}) {final _that = this;
switch (_that) {
case AppEvent_AccountStatus() when accountStatus != null:
return accountStatus(_that.account,_that.state);case AppEvent_SyncProgress() when syncProgress != null:
return syncProgress(_that.account,_that.mailbox,_that.phase,_that.done,_that.total);case AppEvent_OauthUrlReady() when oauthUrlReady != null:
return oauthUrlReady(_that.url);case AppEvent_OauthCompleted() when oauthCompleted != null:
return oauthCompleted(_that.email);case AppEvent_OauthFailed() when oauthFailed != null:
return oauthFailed(_that.reason);case AppEvent_OauthReconnected() when oauthReconnected != null:
return oauthReconnected(_that.account,_that.email);case AppEvent_Other() when other != null:
return other(_that.name);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( PlatformInt64 account,  String state)  accountStatus,required TResult Function( PlatformInt64 account,  String mailbox,  String phase,  int done,  int total)  syncProgress,required TResult Function( String url)  oauthUrlReady,required TResult Function( String email)  oauthCompleted,required TResult Function( String reason)  oauthFailed,required TResult Function( PlatformInt64 account,  String email)  oauthReconnected,required TResult Function( String name)  other,}) {final _that = this;
switch (_that) {
case AppEvent_AccountStatus():
return accountStatus(_that.account,_that.state);case AppEvent_SyncProgress():
return syncProgress(_that.account,_that.mailbox,_that.phase,_that.done,_that.total);case AppEvent_OauthUrlReady():
return oauthUrlReady(_that.url);case AppEvent_OauthCompleted():
return oauthCompleted(_that.email);case AppEvent_OauthFailed():
return oauthFailed(_that.reason);case AppEvent_OauthReconnected():
return oauthReconnected(_that.account,_that.email);case AppEvent_Other():
return other(_that.name);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( PlatformInt64 account,  String state)?  accountStatus,TResult? Function( PlatformInt64 account,  String mailbox,  String phase,  int done,  int total)?  syncProgress,TResult? Function( String url)?  oauthUrlReady,TResult? Function( String email)?  oauthCompleted,TResult? Function( String reason)?  oauthFailed,TResult? Function( PlatformInt64 account,  String email)?  oauthReconnected,TResult? Function( String name)?  other,}) {final _that = this;
switch (_that) {
case AppEvent_AccountStatus() when accountStatus != null:
return accountStatus(_that.account,_that.state);case AppEvent_SyncProgress() when syncProgress != null:
return syncProgress(_that.account,_that.mailbox,_that.phase,_that.done,_that.total);case AppEvent_OauthUrlReady() when oauthUrlReady != null:
return oauthUrlReady(_that.url);case AppEvent_OauthCompleted() when oauthCompleted != null:
return oauthCompleted(_that.email);case AppEvent_OauthFailed() when oauthFailed != null:
return oauthFailed(_that.reason);case AppEvent_OauthReconnected() when oauthReconnected != null:
return oauthReconnected(_that.account,_that.email);case AppEvent_Other() when other != null:
return other(_that.name);case _:
  return null;

}
}

}

/// @nodoc


class AppEvent_AccountStatus extends AppEvent {
  const AppEvent_AccountStatus({required this.account, required this.state}): super._();
  

/// Row id of the account.
 final  PlatformInt64 account;
/// Sync state name (`SyncState` snake_case).
 final  String state;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_AccountStatusCopyWith<AppEvent_AccountStatus> get copyWith => _$AppEvent_AccountStatusCopyWithImpl<AppEvent_AccountStatus>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_AccountStatus&&(identical(other.account, account) || other.account == account)&&(identical(other.state, state) || other.state == state));
}


@override
int get hashCode => Object.hash(runtimeType,account,state);

@override
String toString() {
  return 'AppEvent.accountStatus(account: $account, state: $state)';
}


}

/// @nodoc
abstract mixin class $AppEvent_AccountStatusCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_AccountStatusCopyWith(AppEvent_AccountStatus value, $Res Function(AppEvent_AccountStatus) _then) = _$AppEvent_AccountStatusCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 account, String state
});




}
/// @nodoc
class _$AppEvent_AccountStatusCopyWithImpl<$Res>
    implements $AppEvent_AccountStatusCopyWith<$Res> {
  _$AppEvent_AccountStatusCopyWithImpl(this._self, this._then);

  final AppEvent_AccountStatus _self;
  final $Res Function(AppEvent_AccountStatus) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? account = null,Object? state = null,}) {
  return _then(AppEvent_AccountStatus(
account: null == account ? _self.account : account // ignore: cast_nullable_to_non_nullable
as PlatformInt64,state: null == state ? _self.state : state // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class AppEvent_SyncProgress extends AppEvent {
  const AppEvent_SyncProgress({required this.account, required this.mailbox, required this.phase, required this.done, required this.total}): super._();
  

/// Row id of the account.
 final  PlatformInt64 account;
/// Mailbox the batch belongs to.
 final  String mailbox;
/// Sync phase name (currently `initial`).
 final  String phase;
/// Mailboxes completed so far.
 final  int done;
/// Total mailboxes in this sync.
 final  int total;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_SyncProgressCopyWith<AppEvent_SyncProgress> get copyWith => _$AppEvent_SyncProgressCopyWithImpl<AppEvent_SyncProgress>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_SyncProgress&&(identical(other.account, account) || other.account == account)&&(identical(other.mailbox, mailbox) || other.mailbox == mailbox)&&(identical(other.phase, phase) || other.phase == phase)&&(identical(other.done, done) || other.done == done)&&(identical(other.total, total) || other.total == total));
}


@override
int get hashCode => Object.hash(runtimeType,account,mailbox,phase,done,total);

@override
String toString() {
  return 'AppEvent.syncProgress(account: $account, mailbox: $mailbox, phase: $phase, done: $done, total: $total)';
}


}

/// @nodoc
abstract mixin class $AppEvent_SyncProgressCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_SyncProgressCopyWith(AppEvent_SyncProgress value, $Res Function(AppEvent_SyncProgress) _then) = _$AppEvent_SyncProgressCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 account, String mailbox, String phase, int done, int total
});




}
/// @nodoc
class _$AppEvent_SyncProgressCopyWithImpl<$Res>
    implements $AppEvent_SyncProgressCopyWith<$Res> {
  _$AppEvent_SyncProgressCopyWithImpl(this._self, this._then);

  final AppEvent_SyncProgress _self;
  final $Res Function(AppEvent_SyncProgress) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? account = null,Object? mailbox = null,Object? phase = null,Object? done = null,Object? total = null,}) {
  return _then(AppEvent_SyncProgress(
account: null == account ? _self.account : account // ignore: cast_nullable_to_non_nullable
as PlatformInt64,mailbox: null == mailbox ? _self.mailbox : mailbox // ignore: cast_nullable_to_non_nullable
as String,phase: null == phase ? _self.phase : phase // ignore: cast_nullable_to_non_nullable
as String,done: null == done ? _self.done : done // ignore: cast_nullable_to_non_nullable
as int,total: null == total ? _self.total : total // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class AppEvent_OauthUrlReady extends AppEvent {
  const AppEvent_OauthUrlReady({required this.url}): super._();
  

/// URL to open in the system browser.
 final  String url;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_OauthUrlReadyCopyWith<AppEvent_OauthUrlReady> get copyWith => _$AppEvent_OauthUrlReadyCopyWithImpl<AppEvent_OauthUrlReady>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_OauthUrlReady&&(identical(other.url, url) || other.url == url));
}


@override
int get hashCode => Object.hash(runtimeType,url);

@override
String toString() {
  return 'AppEvent.oauthUrlReady(url: $url)';
}


}

/// @nodoc
abstract mixin class $AppEvent_OauthUrlReadyCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_OauthUrlReadyCopyWith(AppEvent_OauthUrlReady value, $Res Function(AppEvent_OauthUrlReady) _then) = _$AppEvent_OauthUrlReadyCopyWithImpl;
@useResult
$Res call({
 String url
});




}
/// @nodoc
class _$AppEvent_OauthUrlReadyCopyWithImpl<$Res>
    implements $AppEvent_OauthUrlReadyCopyWith<$Res> {
  _$AppEvent_OauthUrlReadyCopyWithImpl(this._self, this._then);

  final AppEvent_OauthUrlReady _self;
  final $Res Function(AppEvent_OauthUrlReady) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? url = null,}) {
  return _then(AppEvent_OauthUrlReady(
url: null == url ? _self.url : url // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class AppEvent_OauthCompleted extends AppEvent {
  const AppEvent_OauthCompleted({required this.email}): super._();
  

/// Email address that just signed in.
 final  String email;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_OauthCompletedCopyWith<AppEvent_OauthCompleted> get copyWith => _$AppEvent_OauthCompletedCopyWithImpl<AppEvent_OauthCompleted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_OauthCompleted&&(identical(other.email, email) || other.email == email));
}


@override
int get hashCode => Object.hash(runtimeType,email);

@override
String toString() {
  return 'AppEvent.oauthCompleted(email: $email)';
}


}

/// @nodoc
abstract mixin class $AppEvent_OauthCompletedCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_OauthCompletedCopyWith(AppEvent_OauthCompleted value, $Res Function(AppEvent_OauthCompleted) _then) = _$AppEvent_OauthCompletedCopyWithImpl;
@useResult
$Res call({
 String email
});




}
/// @nodoc
class _$AppEvent_OauthCompletedCopyWithImpl<$Res>
    implements $AppEvent_OauthCompletedCopyWith<$Res> {
  _$AppEvent_OauthCompletedCopyWithImpl(this._self, this._then);

  final AppEvent_OauthCompleted _self;
  final $Res Function(AppEvent_OauthCompleted) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? email = null,}) {
  return _then(AppEvent_OauthCompleted(
email: null == email ? _self.email : email // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class AppEvent_OauthFailed extends AppEvent {
  const AppEvent_OauthFailed({required this.reason}): super._();
  

/// Why the handshake failed.
 final  String reason;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_OauthFailedCopyWith<AppEvent_OauthFailed> get copyWith => _$AppEvent_OauthFailedCopyWithImpl<AppEvent_OauthFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_OauthFailed&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode => Object.hash(runtimeType,reason);

@override
String toString() {
  return 'AppEvent.oauthFailed(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $AppEvent_OauthFailedCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_OauthFailedCopyWith(AppEvent_OauthFailed value, $Res Function(AppEvent_OauthFailed) _then) = _$AppEvent_OauthFailedCopyWithImpl;
@useResult
$Res call({
 String reason
});




}
/// @nodoc
class _$AppEvent_OauthFailedCopyWithImpl<$Res>
    implements $AppEvent_OauthFailedCopyWith<$Res> {
  _$AppEvent_OauthFailedCopyWithImpl(this._self, this._then);

  final AppEvent_OauthFailed _self;
  final $Res Function(AppEvent_OauthFailed) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(AppEvent_OauthFailed(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class AppEvent_OauthReconnected extends AppEvent {
  const AppEvent_OauthReconnected({required this.account, required this.email}): super._();
  

/// Row id of the reconnected account.
 final  PlatformInt64 account;
/// Email address the provider returned.
 final  String email;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_OauthReconnectedCopyWith<AppEvent_OauthReconnected> get copyWith => _$AppEvent_OauthReconnectedCopyWithImpl<AppEvent_OauthReconnected>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_OauthReconnected&&(identical(other.account, account) || other.account == account)&&(identical(other.email, email) || other.email == email));
}


@override
int get hashCode => Object.hash(runtimeType,account,email);

@override
String toString() {
  return 'AppEvent.oauthReconnected(account: $account, email: $email)';
}


}

/// @nodoc
abstract mixin class $AppEvent_OauthReconnectedCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_OauthReconnectedCopyWith(AppEvent_OauthReconnected value, $Res Function(AppEvent_OauthReconnected) _then) = _$AppEvent_OauthReconnectedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 account, String email
});




}
/// @nodoc
class _$AppEvent_OauthReconnectedCopyWithImpl<$Res>
    implements $AppEvent_OauthReconnectedCopyWith<$Res> {
  _$AppEvent_OauthReconnectedCopyWithImpl(this._self, this._then);

  final AppEvent_OauthReconnected _self;
  final $Res Function(AppEvent_OauthReconnected) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? account = null,Object? email = null,}) {
  return _then(AppEvent_OauthReconnected(
account: null == account ? _self.account : account // ignore: cast_nullable_to_non_nullable
as PlatformInt64,email: null == email ? _self.email : email // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class AppEvent_Other extends AppEvent {
  const AppEvent_Other({required this.name}): super._();
  

/// The `event` field from the wire frame.
 final  String name;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AppEvent_OtherCopyWith<AppEvent_Other> get copyWith => _$AppEvent_OtherCopyWithImpl<AppEvent_Other>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AppEvent_Other&&(identical(other.name, name) || other.name == name));
}


@override
int get hashCode => Object.hash(runtimeType,name);

@override
String toString() {
  return 'AppEvent.other(name: $name)';
}


}

/// @nodoc
abstract mixin class $AppEvent_OtherCopyWith<$Res> implements $AppEventCopyWith<$Res> {
  factory $AppEvent_OtherCopyWith(AppEvent_Other value, $Res Function(AppEvent_Other) _then) = _$AppEvent_OtherCopyWithImpl;
@useResult
$Res call({
 String name
});




}
/// @nodoc
class _$AppEvent_OtherCopyWithImpl<$Res>
    implements $AppEvent_OtherCopyWith<$Res> {
  _$AppEvent_OtherCopyWithImpl(this._self, this._then);

  final AppEvent_Other _self;
  final $Res Function(AppEvent_Other) _then;

/// Create a copy of AppEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? name = null,}) {
  return _then(AppEvent_Other(
name: null == name ? _self.name : name // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

// dart format on
