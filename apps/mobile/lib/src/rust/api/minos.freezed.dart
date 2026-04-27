// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'minos.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$AuthStateFrame {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AuthStateFrame);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'AuthStateFrame()';
}


}

/// @nodoc
class $AuthStateFrameCopyWith<$Res>  {
$AuthStateFrameCopyWith(AuthStateFrame _, $Res Function(AuthStateFrame) __);
}


/// Adds pattern-matching-related methods to [AuthStateFrame].
extension AuthStateFramePatterns on AuthStateFrame {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( AuthStateFrame_Unauthenticated value)?  unauthenticated,TResult Function( AuthStateFrame_Authenticated value)?  authenticated,TResult Function( AuthStateFrame_Refreshing value)?  refreshing,TResult Function( AuthStateFrame_RefreshFailed value)?  refreshFailed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated() when unauthenticated != null:
return unauthenticated(_that);case AuthStateFrame_Authenticated() when authenticated != null:
return authenticated(_that);case AuthStateFrame_Refreshing() when refreshing != null:
return refreshing(_that);case AuthStateFrame_RefreshFailed() when refreshFailed != null:
return refreshFailed(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( AuthStateFrame_Unauthenticated value)  unauthenticated,required TResult Function( AuthStateFrame_Authenticated value)  authenticated,required TResult Function( AuthStateFrame_Refreshing value)  refreshing,required TResult Function( AuthStateFrame_RefreshFailed value)  refreshFailed,}){
final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated():
return unauthenticated(_that);case AuthStateFrame_Authenticated():
return authenticated(_that);case AuthStateFrame_Refreshing():
return refreshing(_that);case AuthStateFrame_RefreshFailed():
return refreshFailed(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( AuthStateFrame_Unauthenticated value)?  unauthenticated,TResult? Function( AuthStateFrame_Authenticated value)?  authenticated,TResult? Function( AuthStateFrame_Refreshing value)?  refreshing,TResult? Function( AuthStateFrame_RefreshFailed value)?  refreshFailed,}){
final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated() when unauthenticated != null:
return unauthenticated(_that);case AuthStateFrame_Authenticated() when authenticated != null:
return authenticated(_that);case AuthStateFrame_Refreshing() when refreshing != null:
return refreshing(_that);case AuthStateFrame_RefreshFailed() when refreshFailed != null:
return refreshFailed(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  unauthenticated,TResult Function( AuthSummary account)?  authenticated,TResult Function()?  refreshing,TResult Function( MinosError error)?  refreshFailed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated() when unauthenticated != null:
return unauthenticated();case AuthStateFrame_Authenticated() when authenticated != null:
return authenticated(_that.account);case AuthStateFrame_Refreshing() when refreshing != null:
return refreshing();case AuthStateFrame_RefreshFailed() when refreshFailed != null:
return refreshFailed(_that.error);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  unauthenticated,required TResult Function( AuthSummary account)  authenticated,required TResult Function()  refreshing,required TResult Function( MinosError error)  refreshFailed,}) {final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated():
return unauthenticated();case AuthStateFrame_Authenticated():
return authenticated(_that.account);case AuthStateFrame_Refreshing():
return refreshing();case AuthStateFrame_RefreshFailed():
return refreshFailed(_that.error);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  unauthenticated,TResult? Function( AuthSummary account)?  authenticated,TResult? Function()?  refreshing,TResult? Function( MinosError error)?  refreshFailed,}) {final _that = this;
switch (_that) {
case AuthStateFrame_Unauthenticated() when unauthenticated != null:
return unauthenticated();case AuthStateFrame_Authenticated() when authenticated != null:
return authenticated(_that.account);case AuthStateFrame_Refreshing() when refreshing != null:
return refreshing();case AuthStateFrame_RefreshFailed() when refreshFailed != null:
return refreshFailed(_that.error);case _:
  return null;

}
}

}

/// @nodoc


class AuthStateFrame_Unauthenticated extends AuthStateFrame {
  const AuthStateFrame_Unauthenticated(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AuthStateFrame_Unauthenticated);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'AuthStateFrame.unauthenticated()';
}


}




/// @nodoc


class AuthStateFrame_Authenticated extends AuthStateFrame {
  const AuthStateFrame_Authenticated({required this.account}): super._();
  

 final  AuthSummary account;

/// Create a copy of AuthStateFrame
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AuthStateFrame_AuthenticatedCopyWith<AuthStateFrame_Authenticated> get copyWith => _$AuthStateFrame_AuthenticatedCopyWithImpl<AuthStateFrame_Authenticated>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AuthStateFrame_Authenticated&&(identical(other.account, account) || other.account == account));
}


@override
int get hashCode => Object.hash(runtimeType,account);

@override
String toString() {
  return 'AuthStateFrame.authenticated(account: $account)';
}


}

/// @nodoc
abstract mixin class $AuthStateFrame_AuthenticatedCopyWith<$Res> implements $AuthStateFrameCopyWith<$Res> {
  factory $AuthStateFrame_AuthenticatedCopyWith(AuthStateFrame_Authenticated value, $Res Function(AuthStateFrame_Authenticated) _then) = _$AuthStateFrame_AuthenticatedCopyWithImpl;
@useResult
$Res call({
 AuthSummary account
});




}
/// @nodoc
class _$AuthStateFrame_AuthenticatedCopyWithImpl<$Res>
    implements $AuthStateFrame_AuthenticatedCopyWith<$Res> {
  _$AuthStateFrame_AuthenticatedCopyWithImpl(this._self, this._then);

  final AuthStateFrame_Authenticated _self;
  final $Res Function(AuthStateFrame_Authenticated) _then;

/// Create a copy of AuthStateFrame
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? account = null,}) {
  return _then(AuthStateFrame_Authenticated(
account: null == account ? _self.account : account // ignore: cast_nullable_to_non_nullable
as AuthSummary,
  ));
}


}

/// @nodoc


class AuthStateFrame_Refreshing extends AuthStateFrame {
  const AuthStateFrame_Refreshing(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AuthStateFrame_Refreshing);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'AuthStateFrame.refreshing()';
}


}




/// @nodoc


class AuthStateFrame_RefreshFailed extends AuthStateFrame {
  const AuthStateFrame_RefreshFailed({required this.error}): super._();
  

 final  MinosError error;

/// Create a copy of AuthStateFrame
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$AuthStateFrame_RefreshFailedCopyWith<AuthStateFrame_RefreshFailed> get copyWith => _$AuthStateFrame_RefreshFailedCopyWithImpl<AuthStateFrame_RefreshFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is AuthStateFrame_RefreshFailed&&(identical(other.error, error) || other.error == error));
}


@override
int get hashCode => Object.hash(runtimeType,error);

@override
String toString() {
  return 'AuthStateFrame.refreshFailed(error: $error)';
}


}

/// @nodoc
abstract mixin class $AuthStateFrame_RefreshFailedCopyWith<$Res> implements $AuthStateFrameCopyWith<$Res> {
  factory $AuthStateFrame_RefreshFailedCopyWith(AuthStateFrame_RefreshFailed value, $Res Function(AuthStateFrame_RefreshFailed) _then) = _$AuthStateFrame_RefreshFailedCopyWithImpl;
@useResult
$Res call({
 MinosError error
});


$MinosErrorCopyWith<$Res> get error;

}
/// @nodoc
class _$AuthStateFrame_RefreshFailedCopyWithImpl<$Res>
    implements $AuthStateFrame_RefreshFailedCopyWith<$Res> {
  _$AuthStateFrame_RefreshFailedCopyWithImpl(this._self, this._then);

  final AuthStateFrame_RefreshFailed _self;
  final $Res Function(AuthStateFrame_RefreshFailed) _then;

/// Create a copy of AuthStateFrame
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? error = null,}) {
  return _then(AuthStateFrame_RefreshFailed(
error: null == error ? _self.error : error // ignore: cast_nullable_to_non_nullable
as MinosError,
  ));
}

/// Create a copy of AuthStateFrame
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$MinosErrorCopyWith<$Res> get error {
  
  return $MinosErrorCopyWith<$Res>(_self.error, (value) {
    return _then(_self.copyWith(error: value));
  });
}
}

/// @nodoc
mixin _$ConnectionState {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ConnectionState);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ConnectionState()';
}


}

/// @nodoc
class $ConnectionStateCopyWith<$Res>  {
$ConnectionStateCopyWith(ConnectionState _, $Res Function(ConnectionState) __);
}


/// Adds pattern-matching-related methods to [ConnectionState].
extension ConnectionStatePatterns on ConnectionState {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ConnectionState_Disconnected value)?  disconnected,TResult Function( ConnectionState_Pairing value)?  pairing,TResult Function( ConnectionState_Connected value)?  connected,TResult Function( ConnectionState_Reconnecting value)?  reconnecting,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ConnectionState_Disconnected() when disconnected != null:
return disconnected(_that);case ConnectionState_Pairing() when pairing != null:
return pairing(_that);case ConnectionState_Connected() when connected != null:
return connected(_that);case ConnectionState_Reconnecting() when reconnecting != null:
return reconnecting(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ConnectionState_Disconnected value)  disconnected,required TResult Function( ConnectionState_Pairing value)  pairing,required TResult Function( ConnectionState_Connected value)  connected,required TResult Function( ConnectionState_Reconnecting value)  reconnecting,}){
final _that = this;
switch (_that) {
case ConnectionState_Disconnected():
return disconnected(_that);case ConnectionState_Pairing():
return pairing(_that);case ConnectionState_Connected():
return connected(_that);case ConnectionState_Reconnecting():
return reconnecting(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ConnectionState_Disconnected value)?  disconnected,TResult? Function( ConnectionState_Pairing value)?  pairing,TResult? Function( ConnectionState_Connected value)?  connected,TResult? Function( ConnectionState_Reconnecting value)?  reconnecting,}){
final _that = this;
switch (_that) {
case ConnectionState_Disconnected() when disconnected != null:
return disconnected(_that);case ConnectionState_Pairing() when pairing != null:
return pairing(_that);case ConnectionState_Connected() when connected != null:
return connected(_that);case ConnectionState_Reconnecting() when reconnecting != null:
return reconnecting(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  disconnected,TResult Function()?  pairing,TResult Function()?  connected,TResult Function( int attempt)?  reconnecting,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ConnectionState_Disconnected() when disconnected != null:
return disconnected();case ConnectionState_Pairing() when pairing != null:
return pairing();case ConnectionState_Connected() when connected != null:
return connected();case ConnectionState_Reconnecting() when reconnecting != null:
return reconnecting(_that.attempt);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  disconnected,required TResult Function()  pairing,required TResult Function()  connected,required TResult Function( int attempt)  reconnecting,}) {final _that = this;
switch (_that) {
case ConnectionState_Disconnected():
return disconnected();case ConnectionState_Pairing():
return pairing();case ConnectionState_Connected():
return connected();case ConnectionState_Reconnecting():
return reconnecting(_that.attempt);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  disconnected,TResult? Function()?  pairing,TResult? Function()?  connected,TResult? Function( int attempt)?  reconnecting,}) {final _that = this;
switch (_that) {
case ConnectionState_Disconnected() when disconnected != null:
return disconnected();case ConnectionState_Pairing() when pairing != null:
return pairing();case ConnectionState_Connected() when connected != null:
return connected();case ConnectionState_Reconnecting() when reconnecting != null:
return reconnecting(_that.attempt);case _:
  return null;

}
}

}

/// @nodoc


class ConnectionState_Disconnected extends ConnectionState {
  const ConnectionState_Disconnected(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ConnectionState_Disconnected);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ConnectionState.disconnected()';
}


}




/// @nodoc


class ConnectionState_Pairing extends ConnectionState {
  const ConnectionState_Pairing(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ConnectionState_Pairing);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ConnectionState.pairing()';
}


}




/// @nodoc


class ConnectionState_Connected extends ConnectionState {
  const ConnectionState_Connected(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ConnectionState_Connected);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ConnectionState.connected()';
}


}




/// @nodoc


class ConnectionState_Reconnecting extends ConnectionState {
  const ConnectionState_Reconnecting({required this.attempt}): super._();
  

 final  int attempt;

/// Create a copy of ConnectionState
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ConnectionState_ReconnectingCopyWith<ConnectionState_Reconnecting> get copyWith => _$ConnectionState_ReconnectingCopyWithImpl<ConnectionState_Reconnecting>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ConnectionState_Reconnecting&&(identical(other.attempt, attempt) || other.attempt == attempt));
}


@override
int get hashCode => Object.hash(runtimeType,attempt);

@override
String toString() {
  return 'ConnectionState.reconnecting(attempt: $attempt)';
}


}

/// @nodoc
abstract mixin class $ConnectionState_ReconnectingCopyWith<$Res> implements $ConnectionStateCopyWith<$Res> {
  factory $ConnectionState_ReconnectingCopyWith(ConnectionState_Reconnecting value, $Res Function(ConnectionState_Reconnecting) _then) = _$ConnectionState_ReconnectingCopyWithImpl;
@useResult
$Res call({
 int attempt
});




}
/// @nodoc
class _$ConnectionState_ReconnectingCopyWithImpl<$Res>
    implements $ConnectionState_ReconnectingCopyWith<$Res> {
  _$ConnectionState_ReconnectingCopyWithImpl(this._self, this._then);

  final ConnectionState_Reconnecting _self;
  final $Res Function(ConnectionState_Reconnecting) _then;

/// Create a copy of ConnectionState
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? attempt = null,}) {
  return _then(ConnectionState_Reconnecting(
attempt: null == attempt ? _self.attempt : attempt // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc
mixin _$MinosError {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError()';
}


}

/// @nodoc
class $MinosErrorCopyWith<$Res>  {
$MinosErrorCopyWith(MinosError _, $Res Function(MinosError) __);
}


/// Adds pattern-matching-related methods to [MinosError].
extension MinosErrorPatterns on MinosError {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( MinosError_BindFailed value)?  bindFailed,TResult Function( MinosError_ConnectFailed value)?  connectFailed,TResult Function( MinosError_Disconnected value)?  disconnected,TResult Function( MinosError_PairingTokenInvalid value)?  pairingTokenInvalid,TResult Function( MinosError_PairingStateMismatch value)?  pairingStateMismatch,TResult Function( MinosError_DeviceNotTrusted value)?  deviceNotTrusted,TResult Function( MinosError_StoreIo value)?  storeIo,TResult Function( MinosError_StoreCorrupt value)?  storeCorrupt,TResult Function( MinosError_CliProbeTimeout value)?  cliProbeTimeout,TResult Function( MinosError_CliProbeFailed value)?  cliProbeFailed,TResult Function( MinosError_RpcCallFailed value)?  rpcCallFailed,TResult Function( MinosError_Unauthorized value)?  unauthorized,TResult Function( MinosError_ConnectionStateMismatch value)?  connectionStateMismatch,TResult Function( MinosError_EnvelopeVersionUnsupported value)?  envelopeVersionUnsupported,TResult Function( MinosError_PeerOffline value)?  peerOffline,TResult Function( MinosError_BackendInternal value)?  backendInternal,TResult Function( MinosError_CfAuthFailed value)?  cfAuthFailed,TResult Function( MinosError_CodexSpawnFailed value)?  codexSpawnFailed,TResult Function( MinosError_CodexConnectFailed value)?  codexConnectFailed,TResult Function( MinosError_CodexProtocolError value)?  codexProtocolError,TResult Function( MinosError_AgentAlreadyRunning value)?  agentAlreadyRunning,TResult Function( MinosError_AgentNotRunning value)?  agentNotRunning,TResult Function( MinosError_AgentNotSupported value)?  agentNotSupported,TResult Function( MinosError_AgentSessionIdMismatch value)?  agentSessionIdMismatch,TResult Function( MinosError_CfAccessMisconfigured value)?  cfAccessMisconfigured,TResult Function( MinosError_IngestSeqConflict value)?  ingestSeqConflict,TResult Function( MinosError_ThreadNotFound value)?  threadNotFound,TResult Function( MinosError_TranslationNotImplemented value)?  translationNotImplemented,TResult Function( MinosError_TranslationFailed value)?  translationFailed,TResult Function( MinosError_PairingQrVersionUnsupported value)?  pairingQrVersionUnsupported,TResult Function( MinosError_Timeout value)?  timeout,TResult Function( MinosError_NotConnected value)?  notConnected,TResult Function( MinosError_RequestDropped value)?  requestDropped,TResult Function( MinosError_AuthRefreshFailed value)?  authRefreshFailed,TResult Function( MinosError_EmailTaken value)?  emailTaken,TResult Function( MinosError_WeakPassword value)?  weakPassword,TResult Function( MinosError_RateLimited value)?  rateLimited,TResult Function( MinosError_InvalidCredentials value)?  invalidCredentials,TResult Function( MinosError_AgentStartFailed value)?  agentStartFailed,TResult Function( MinosError_PairingTokenExpired value)?  pairingTokenExpired,required TResult orElse(),}){
final _that = this;
switch (_that) {
case MinosError_BindFailed() when bindFailed != null:
return bindFailed(_that);case MinosError_ConnectFailed() when connectFailed != null:
return connectFailed(_that);case MinosError_Disconnected() when disconnected != null:
return disconnected(_that);case MinosError_PairingTokenInvalid() when pairingTokenInvalid != null:
return pairingTokenInvalid(_that);case MinosError_PairingStateMismatch() when pairingStateMismatch != null:
return pairingStateMismatch(_that);case MinosError_DeviceNotTrusted() when deviceNotTrusted != null:
return deviceNotTrusted(_that);case MinosError_StoreIo() when storeIo != null:
return storeIo(_that);case MinosError_StoreCorrupt() when storeCorrupt != null:
return storeCorrupt(_that);case MinosError_CliProbeTimeout() when cliProbeTimeout != null:
return cliProbeTimeout(_that);case MinosError_CliProbeFailed() when cliProbeFailed != null:
return cliProbeFailed(_that);case MinosError_RpcCallFailed() when rpcCallFailed != null:
return rpcCallFailed(_that);case MinosError_Unauthorized() when unauthorized != null:
return unauthorized(_that);case MinosError_ConnectionStateMismatch() when connectionStateMismatch != null:
return connectionStateMismatch(_that);case MinosError_EnvelopeVersionUnsupported() when envelopeVersionUnsupported != null:
return envelopeVersionUnsupported(_that);case MinosError_PeerOffline() when peerOffline != null:
return peerOffline(_that);case MinosError_BackendInternal() when backendInternal != null:
return backendInternal(_that);case MinosError_CfAuthFailed() when cfAuthFailed != null:
return cfAuthFailed(_that);case MinosError_CodexSpawnFailed() when codexSpawnFailed != null:
return codexSpawnFailed(_that);case MinosError_CodexConnectFailed() when codexConnectFailed != null:
return codexConnectFailed(_that);case MinosError_CodexProtocolError() when codexProtocolError != null:
return codexProtocolError(_that);case MinosError_AgentAlreadyRunning() when agentAlreadyRunning != null:
return agentAlreadyRunning(_that);case MinosError_AgentNotRunning() when agentNotRunning != null:
return agentNotRunning(_that);case MinosError_AgentNotSupported() when agentNotSupported != null:
return agentNotSupported(_that);case MinosError_AgentSessionIdMismatch() when agentSessionIdMismatch != null:
return agentSessionIdMismatch(_that);case MinosError_CfAccessMisconfigured() when cfAccessMisconfigured != null:
return cfAccessMisconfigured(_that);case MinosError_IngestSeqConflict() when ingestSeqConflict != null:
return ingestSeqConflict(_that);case MinosError_ThreadNotFound() when threadNotFound != null:
return threadNotFound(_that);case MinosError_TranslationNotImplemented() when translationNotImplemented != null:
return translationNotImplemented(_that);case MinosError_TranslationFailed() when translationFailed != null:
return translationFailed(_that);case MinosError_PairingQrVersionUnsupported() when pairingQrVersionUnsupported != null:
return pairingQrVersionUnsupported(_that);case MinosError_Timeout() when timeout != null:
return timeout(_that);case MinosError_NotConnected() when notConnected != null:
return notConnected(_that);case MinosError_RequestDropped() when requestDropped != null:
return requestDropped(_that);case MinosError_AuthRefreshFailed() when authRefreshFailed != null:
return authRefreshFailed(_that);case MinosError_EmailTaken() when emailTaken != null:
return emailTaken(_that);case MinosError_WeakPassword() when weakPassword != null:
return weakPassword(_that);case MinosError_RateLimited() when rateLimited != null:
return rateLimited(_that);case MinosError_InvalidCredentials() when invalidCredentials != null:
return invalidCredentials(_that);case MinosError_AgentStartFailed() when agentStartFailed != null:
return agentStartFailed(_that);case MinosError_PairingTokenExpired() when pairingTokenExpired != null:
return pairingTokenExpired(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( MinosError_BindFailed value)  bindFailed,required TResult Function( MinosError_ConnectFailed value)  connectFailed,required TResult Function( MinosError_Disconnected value)  disconnected,required TResult Function( MinosError_PairingTokenInvalid value)  pairingTokenInvalid,required TResult Function( MinosError_PairingStateMismatch value)  pairingStateMismatch,required TResult Function( MinosError_DeviceNotTrusted value)  deviceNotTrusted,required TResult Function( MinosError_StoreIo value)  storeIo,required TResult Function( MinosError_StoreCorrupt value)  storeCorrupt,required TResult Function( MinosError_CliProbeTimeout value)  cliProbeTimeout,required TResult Function( MinosError_CliProbeFailed value)  cliProbeFailed,required TResult Function( MinosError_RpcCallFailed value)  rpcCallFailed,required TResult Function( MinosError_Unauthorized value)  unauthorized,required TResult Function( MinosError_ConnectionStateMismatch value)  connectionStateMismatch,required TResult Function( MinosError_EnvelopeVersionUnsupported value)  envelopeVersionUnsupported,required TResult Function( MinosError_PeerOffline value)  peerOffline,required TResult Function( MinosError_BackendInternal value)  backendInternal,required TResult Function( MinosError_CfAuthFailed value)  cfAuthFailed,required TResult Function( MinosError_CodexSpawnFailed value)  codexSpawnFailed,required TResult Function( MinosError_CodexConnectFailed value)  codexConnectFailed,required TResult Function( MinosError_CodexProtocolError value)  codexProtocolError,required TResult Function( MinosError_AgentAlreadyRunning value)  agentAlreadyRunning,required TResult Function( MinosError_AgentNotRunning value)  agentNotRunning,required TResult Function( MinosError_AgentNotSupported value)  agentNotSupported,required TResult Function( MinosError_AgentSessionIdMismatch value)  agentSessionIdMismatch,required TResult Function( MinosError_CfAccessMisconfigured value)  cfAccessMisconfigured,required TResult Function( MinosError_IngestSeqConflict value)  ingestSeqConflict,required TResult Function( MinosError_ThreadNotFound value)  threadNotFound,required TResult Function( MinosError_TranslationNotImplemented value)  translationNotImplemented,required TResult Function( MinosError_TranslationFailed value)  translationFailed,required TResult Function( MinosError_PairingQrVersionUnsupported value)  pairingQrVersionUnsupported,required TResult Function( MinosError_Timeout value)  timeout,required TResult Function( MinosError_NotConnected value)  notConnected,required TResult Function( MinosError_RequestDropped value)  requestDropped,required TResult Function( MinosError_AuthRefreshFailed value)  authRefreshFailed,required TResult Function( MinosError_EmailTaken value)  emailTaken,required TResult Function( MinosError_WeakPassword value)  weakPassword,required TResult Function( MinosError_RateLimited value)  rateLimited,required TResult Function( MinosError_InvalidCredentials value)  invalidCredentials,required TResult Function( MinosError_AgentStartFailed value)  agentStartFailed,required TResult Function( MinosError_PairingTokenExpired value)  pairingTokenExpired,}){
final _that = this;
switch (_that) {
case MinosError_BindFailed():
return bindFailed(_that);case MinosError_ConnectFailed():
return connectFailed(_that);case MinosError_Disconnected():
return disconnected(_that);case MinosError_PairingTokenInvalid():
return pairingTokenInvalid(_that);case MinosError_PairingStateMismatch():
return pairingStateMismatch(_that);case MinosError_DeviceNotTrusted():
return deviceNotTrusted(_that);case MinosError_StoreIo():
return storeIo(_that);case MinosError_StoreCorrupt():
return storeCorrupt(_that);case MinosError_CliProbeTimeout():
return cliProbeTimeout(_that);case MinosError_CliProbeFailed():
return cliProbeFailed(_that);case MinosError_RpcCallFailed():
return rpcCallFailed(_that);case MinosError_Unauthorized():
return unauthorized(_that);case MinosError_ConnectionStateMismatch():
return connectionStateMismatch(_that);case MinosError_EnvelopeVersionUnsupported():
return envelopeVersionUnsupported(_that);case MinosError_PeerOffline():
return peerOffline(_that);case MinosError_BackendInternal():
return backendInternal(_that);case MinosError_CfAuthFailed():
return cfAuthFailed(_that);case MinosError_CodexSpawnFailed():
return codexSpawnFailed(_that);case MinosError_CodexConnectFailed():
return codexConnectFailed(_that);case MinosError_CodexProtocolError():
return codexProtocolError(_that);case MinosError_AgentAlreadyRunning():
return agentAlreadyRunning(_that);case MinosError_AgentNotRunning():
return agentNotRunning(_that);case MinosError_AgentNotSupported():
return agentNotSupported(_that);case MinosError_AgentSessionIdMismatch():
return agentSessionIdMismatch(_that);case MinosError_CfAccessMisconfigured():
return cfAccessMisconfigured(_that);case MinosError_IngestSeqConflict():
return ingestSeqConflict(_that);case MinosError_ThreadNotFound():
return threadNotFound(_that);case MinosError_TranslationNotImplemented():
return translationNotImplemented(_that);case MinosError_TranslationFailed():
return translationFailed(_that);case MinosError_PairingQrVersionUnsupported():
return pairingQrVersionUnsupported(_that);case MinosError_Timeout():
return timeout(_that);case MinosError_NotConnected():
return notConnected(_that);case MinosError_RequestDropped():
return requestDropped(_that);case MinosError_AuthRefreshFailed():
return authRefreshFailed(_that);case MinosError_EmailTaken():
return emailTaken(_that);case MinosError_WeakPassword():
return weakPassword(_that);case MinosError_RateLimited():
return rateLimited(_that);case MinosError_InvalidCredentials():
return invalidCredentials(_that);case MinosError_AgentStartFailed():
return agentStartFailed(_that);case MinosError_PairingTokenExpired():
return pairingTokenExpired(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( MinosError_BindFailed value)?  bindFailed,TResult? Function( MinosError_ConnectFailed value)?  connectFailed,TResult? Function( MinosError_Disconnected value)?  disconnected,TResult? Function( MinosError_PairingTokenInvalid value)?  pairingTokenInvalid,TResult? Function( MinosError_PairingStateMismatch value)?  pairingStateMismatch,TResult? Function( MinosError_DeviceNotTrusted value)?  deviceNotTrusted,TResult? Function( MinosError_StoreIo value)?  storeIo,TResult? Function( MinosError_StoreCorrupt value)?  storeCorrupt,TResult? Function( MinosError_CliProbeTimeout value)?  cliProbeTimeout,TResult? Function( MinosError_CliProbeFailed value)?  cliProbeFailed,TResult? Function( MinosError_RpcCallFailed value)?  rpcCallFailed,TResult? Function( MinosError_Unauthorized value)?  unauthorized,TResult? Function( MinosError_ConnectionStateMismatch value)?  connectionStateMismatch,TResult? Function( MinosError_EnvelopeVersionUnsupported value)?  envelopeVersionUnsupported,TResult? Function( MinosError_PeerOffline value)?  peerOffline,TResult? Function( MinosError_BackendInternal value)?  backendInternal,TResult? Function( MinosError_CfAuthFailed value)?  cfAuthFailed,TResult? Function( MinosError_CodexSpawnFailed value)?  codexSpawnFailed,TResult? Function( MinosError_CodexConnectFailed value)?  codexConnectFailed,TResult? Function( MinosError_CodexProtocolError value)?  codexProtocolError,TResult? Function( MinosError_AgentAlreadyRunning value)?  agentAlreadyRunning,TResult? Function( MinosError_AgentNotRunning value)?  agentNotRunning,TResult? Function( MinosError_AgentNotSupported value)?  agentNotSupported,TResult? Function( MinosError_AgentSessionIdMismatch value)?  agentSessionIdMismatch,TResult? Function( MinosError_CfAccessMisconfigured value)?  cfAccessMisconfigured,TResult? Function( MinosError_IngestSeqConflict value)?  ingestSeqConflict,TResult? Function( MinosError_ThreadNotFound value)?  threadNotFound,TResult? Function( MinosError_TranslationNotImplemented value)?  translationNotImplemented,TResult? Function( MinosError_TranslationFailed value)?  translationFailed,TResult? Function( MinosError_PairingQrVersionUnsupported value)?  pairingQrVersionUnsupported,TResult? Function( MinosError_Timeout value)?  timeout,TResult? Function( MinosError_NotConnected value)?  notConnected,TResult? Function( MinosError_RequestDropped value)?  requestDropped,TResult? Function( MinosError_AuthRefreshFailed value)?  authRefreshFailed,TResult? Function( MinosError_EmailTaken value)?  emailTaken,TResult? Function( MinosError_WeakPassword value)?  weakPassword,TResult? Function( MinosError_RateLimited value)?  rateLimited,TResult? Function( MinosError_InvalidCredentials value)?  invalidCredentials,TResult? Function( MinosError_AgentStartFailed value)?  agentStartFailed,TResult? Function( MinosError_PairingTokenExpired value)?  pairingTokenExpired,}){
final _that = this;
switch (_that) {
case MinosError_BindFailed() when bindFailed != null:
return bindFailed(_that);case MinosError_ConnectFailed() when connectFailed != null:
return connectFailed(_that);case MinosError_Disconnected() when disconnected != null:
return disconnected(_that);case MinosError_PairingTokenInvalid() when pairingTokenInvalid != null:
return pairingTokenInvalid(_that);case MinosError_PairingStateMismatch() when pairingStateMismatch != null:
return pairingStateMismatch(_that);case MinosError_DeviceNotTrusted() when deviceNotTrusted != null:
return deviceNotTrusted(_that);case MinosError_StoreIo() when storeIo != null:
return storeIo(_that);case MinosError_StoreCorrupt() when storeCorrupt != null:
return storeCorrupt(_that);case MinosError_CliProbeTimeout() when cliProbeTimeout != null:
return cliProbeTimeout(_that);case MinosError_CliProbeFailed() when cliProbeFailed != null:
return cliProbeFailed(_that);case MinosError_RpcCallFailed() when rpcCallFailed != null:
return rpcCallFailed(_that);case MinosError_Unauthorized() when unauthorized != null:
return unauthorized(_that);case MinosError_ConnectionStateMismatch() when connectionStateMismatch != null:
return connectionStateMismatch(_that);case MinosError_EnvelopeVersionUnsupported() when envelopeVersionUnsupported != null:
return envelopeVersionUnsupported(_that);case MinosError_PeerOffline() when peerOffline != null:
return peerOffline(_that);case MinosError_BackendInternal() when backendInternal != null:
return backendInternal(_that);case MinosError_CfAuthFailed() when cfAuthFailed != null:
return cfAuthFailed(_that);case MinosError_CodexSpawnFailed() when codexSpawnFailed != null:
return codexSpawnFailed(_that);case MinosError_CodexConnectFailed() when codexConnectFailed != null:
return codexConnectFailed(_that);case MinosError_CodexProtocolError() when codexProtocolError != null:
return codexProtocolError(_that);case MinosError_AgentAlreadyRunning() when agentAlreadyRunning != null:
return agentAlreadyRunning(_that);case MinosError_AgentNotRunning() when agentNotRunning != null:
return agentNotRunning(_that);case MinosError_AgentNotSupported() when agentNotSupported != null:
return agentNotSupported(_that);case MinosError_AgentSessionIdMismatch() when agentSessionIdMismatch != null:
return agentSessionIdMismatch(_that);case MinosError_CfAccessMisconfigured() when cfAccessMisconfigured != null:
return cfAccessMisconfigured(_that);case MinosError_IngestSeqConflict() when ingestSeqConflict != null:
return ingestSeqConflict(_that);case MinosError_ThreadNotFound() when threadNotFound != null:
return threadNotFound(_that);case MinosError_TranslationNotImplemented() when translationNotImplemented != null:
return translationNotImplemented(_that);case MinosError_TranslationFailed() when translationFailed != null:
return translationFailed(_that);case MinosError_PairingQrVersionUnsupported() when pairingQrVersionUnsupported != null:
return pairingQrVersionUnsupported(_that);case MinosError_Timeout() when timeout != null:
return timeout(_that);case MinosError_NotConnected() when notConnected != null:
return notConnected(_that);case MinosError_RequestDropped() when requestDropped != null:
return requestDropped(_that);case MinosError_AuthRefreshFailed() when authRefreshFailed != null:
return authRefreshFailed(_that);case MinosError_EmailTaken() when emailTaken != null:
return emailTaken(_that);case MinosError_WeakPassword() when weakPassword != null:
return weakPassword(_that);case MinosError_RateLimited() when rateLimited != null:
return rateLimited(_that);case MinosError_InvalidCredentials() when invalidCredentials != null:
return invalidCredentials(_that);case MinosError_AgentStartFailed() when agentStartFailed != null:
return agentStartFailed(_that);case MinosError_PairingTokenExpired() when pairingTokenExpired != null:
return pairingTokenExpired(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String addr,  String message)?  bindFailed,TResult Function( String url,  String message)?  connectFailed,TResult Function( String reason)?  disconnected,TResult Function()?  pairingTokenInvalid,TResult Function( PairingState actual)?  pairingStateMismatch,TResult Function( String deviceId)?  deviceNotTrusted,TResult Function( String path,  String message)?  storeIo,TResult Function( String path,  String message)?  storeCorrupt,TResult Function( String bin,  BigInt timeoutMs)?  cliProbeTimeout,TResult Function( String bin,  String message)?  cliProbeFailed,TResult Function( String method,  String message)?  rpcCallFailed,TResult Function( String reason)?  unauthorized,TResult Function( String expected,  String actual)?  connectionStateMismatch,TResult Function( int version)?  envelopeVersionUnsupported,TResult Function( String peerDeviceId)?  peerOffline,TResult Function( String message)?  backendInternal,TResult Function( String message)?  cfAuthFailed,TResult Function( String message)?  codexSpawnFailed,TResult Function( String url,  String message)?  codexConnectFailed,TResult Function( String method,  String message)?  codexProtocolError,TResult Function()?  agentAlreadyRunning,TResult Function()?  agentNotRunning,TResult Function( AgentName agent)?  agentNotSupported,TResult Function()?  agentSessionIdMismatch,TResult Function( String reason)?  cfAccessMisconfigured,TResult Function( String threadId,  BigInt seq)?  ingestSeqConflict,TResult Function( String threadId)?  threadNotFound,TResult Function( AgentName agent)?  translationNotImplemented,TResult Function( AgentName agent,  String message)?  translationFailed,TResult Function( int version)?  pairingQrVersionUnsupported,TResult Function()?  timeout,TResult Function()?  notConnected,TResult Function()?  requestDropped,TResult Function( String message)?  authRefreshFailed,TResult Function()?  emailTaken,TResult Function()?  weakPassword,TResult Function( int retryAfterS)?  rateLimited,TResult Function()?  invalidCredentials,TResult Function( String reason)?  agentStartFailed,TResult Function()?  pairingTokenExpired,required TResult orElse(),}) {final _that = this;
switch (_that) {
case MinosError_BindFailed() when bindFailed != null:
return bindFailed(_that.addr,_that.message);case MinosError_ConnectFailed() when connectFailed != null:
return connectFailed(_that.url,_that.message);case MinosError_Disconnected() when disconnected != null:
return disconnected(_that.reason);case MinosError_PairingTokenInvalid() when pairingTokenInvalid != null:
return pairingTokenInvalid();case MinosError_PairingStateMismatch() when pairingStateMismatch != null:
return pairingStateMismatch(_that.actual);case MinosError_DeviceNotTrusted() when deviceNotTrusted != null:
return deviceNotTrusted(_that.deviceId);case MinosError_StoreIo() when storeIo != null:
return storeIo(_that.path,_that.message);case MinosError_StoreCorrupt() when storeCorrupt != null:
return storeCorrupt(_that.path,_that.message);case MinosError_CliProbeTimeout() when cliProbeTimeout != null:
return cliProbeTimeout(_that.bin,_that.timeoutMs);case MinosError_CliProbeFailed() when cliProbeFailed != null:
return cliProbeFailed(_that.bin,_that.message);case MinosError_RpcCallFailed() when rpcCallFailed != null:
return rpcCallFailed(_that.method,_that.message);case MinosError_Unauthorized() when unauthorized != null:
return unauthorized(_that.reason);case MinosError_ConnectionStateMismatch() when connectionStateMismatch != null:
return connectionStateMismatch(_that.expected,_that.actual);case MinosError_EnvelopeVersionUnsupported() when envelopeVersionUnsupported != null:
return envelopeVersionUnsupported(_that.version);case MinosError_PeerOffline() when peerOffline != null:
return peerOffline(_that.peerDeviceId);case MinosError_BackendInternal() when backendInternal != null:
return backendInternal(_that.message);case MinosError_CfAuthFailed() when cfAuthFailed != null:
return cfAuthFailed(_that.message);case MinosError_CodexSpawnFailed() when codexSpawnFailed != null:
return codexSpawnFailed(_that.message);case MinosError_CodexConnectFailed() when codexConnectFailed != null:
return codexConnectFailed(_that.url,_that.message);case MinosError_CodexProtocolError() when codexProtocolError != null:
return codexProtocolError(_that.method,_that.message);case MinosError_AgentAlreadyRunning() when agentAlreadyRunning != null:
return agentAlreadyRunning();case MinosError_AgentNotRunning() when agentNotRunning != null:
return agentNotRunning();case MinosError_AgentNotSupported() when agentNotSupported != null:
return agentNotSupported(_that.agent);case MinosError_AgentSessionIdMismatch() when agentSessionIdMismatch != null:
return agentSessionIdMismatch();case MinosError_CfAccessMisconfigured() when cfAccessMisconfigured != null:
return cfAccessMisconfigured(_that.reason);case MinosError_IngestSeqConflict() when ingestSeqConflict != null:
return ingestSeqConflict(_that.threadId,_that.seq);case MinosError_ThreadNotFound() when threadNotFound != null:
return threadNotFound(_that.threadId);case MinosError_TranslationNotImplemented() when translationNotImplemented != null:
return translationNotImplemented(_that.agent);case MinosError_TranslationFailed() when translationFailed != null:
return translationFailed(_that.agent,_that.message);case MinosError_PairingQrVersionUnsupported() when pairingQrVersionUnsupported != null:
return pairingQrVersionUnsupported(_that.version);case MinosError_Timeout() when timeout != null:
return timeout();case MinosError_NotConnected() when notConnected != null:
return notConnected();case MinosError_RequestDropped() when requestDropped != null:
return requestDropped();case MinosError_AuthRefreshFailed() when authRefreshFailed != null:
return authRefreshFailed(_that.message);case MinosError_EmailTaken() when emailTaken != null:
return emailTaken();case MinosError_WeakPassword() when weakPassword != null:
return weakPassword();case MinosError_RateLimited() when rateLimited != null:
return rateLimited(_that.retryAfterS);case MinosError_InvalidCredentials() when invalidCredentials != null:
return invalidCredentials();case MinosError_AgentStartFailed() when agentStartFailed != null:
return agentStartFailed(_that.reason);case MinosError_PairingTokenExpired() when pairingTokenExpired != null:
return pairingTokenExpired();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String addr,  String message)  bindFailed,required TResult Function( String url,  String message)  connectFailed,required TResult Function( String reason)  disconnected,required TResult Function()  pairingTokenInvalid,required TResult Function( PairingState actual)  pairingStateMismatch,required TResult Function( String deviceId)  deviceNotTrusted,required TResult Function( String path,  String message)  storeIo,required TResult Function( String path,  String message)  storeCorrupt,required TResult Function( String bin,  BigInt timeoutMs)  cliProbeTimeout,required TResult Function( String bin,  String message)  cliProbeFailed,required TResult Function( String method,  String message)  rpcCallFailed,required TResult Function( String reason)  unauthorized,required TResult Function( String expected,  String actual)  connectionStateMismatch,required TResult Function( int version)  envelopeVersionUnsupported,required TResult Function( String peerDeviceId)  peerOffline,required TResult Function( String message)  backendInternal,required TResult Function( String message)  cfAuthFailed,required TResult Function( String message)  codexSpawnFailed,required TResult Function( String url,  String message)  codexConnectFailed,required TResult Function( String method,  String message)  codexProtocolError,required TResult Function()  agentAlreadyRunning,required TResult Function()  agentNotRunning,required TResult Function( AgentName agent)  agentNotSupported,required TResult Function()  agentSessionIdMismatch,required TResult Function( String reason)  cfAccessMisconfigured,required TResult Function( String threadId,  BigInt seq)  ingestSeqConflict,required TResult Function( String threadId)  threadNotFound,required TResult Function( AgentName agent)  translationNotImplemented,required TResult Function( AgentName agent,  String message)  translationFailed,required TResult Function( int version)  pairingQrVersionUnsupported,required TResult Function()  timeout,required TResult Function()  notConnected,required TResult Function()  requestDropped,required TResult Function( String message)  authRefreshFailed,required TResult Function()  emailTaken,required TResult Function()  weakPassword,required TResult Function( int retryAfterS)  rateLimited,required TResult Function()  invalidCredentials,required TResult Function( String reason)  agentStartFailed,required TResult Function()  pairingTokenExpired,}) {final _that = this;
switch (_that) {
case MinosError_BindFailed():
return bindFailed(_that.addr,_that.message);case MinosError_ConnectFailed():
return connectFailed(_that.url,_that.message);case MinosError_Disconnected():
return disconnected(_that.reason);case MinosError_PairingTokenInvalid():
return pairingTokenInvalid();case MinosError_PairingStateMismatch():
return pairingStateMismatch(_that.actual);case MinosError_DeviceNotTrusted():
return deviceNotTrusted(_that.deviceId);case MinosError_StoreIo():
return storeIo(_that.path,_that.message);case MinosError_StoreCorrupt():
return storeCorrupt(_that.path,_that.message);case MinosError_CliProbeTimeout():
return cliProbeTimeout(_that.bin,_that.timeoutMs);case MinosError_CliProbeFailed():
return cliProbeFailed(_that.bin,_that.message);case MinosError_RpcCallFailed():
return rpcCallFailed(_that.method,_that.message);case MinosError_Unauthorized():
return unauthorized(_that.reason);case MinosError_ConnectionStateMismatch():
return connectionStateMismatch(_that.expected,_that.actual);case MinosError_EnvelopeVersionUnsupported():
return envelopeVersionUnsupported(_that.version);case MinosError_PeerOffline():
return peerOffline(_that.peerDeviceId);case MinosError_BackendInternal():
return backendInternal(_that.message);case MinosError_CfAuthFailed():
return cfAuthFailed(_that.message);case MinosError_CodexSpawnFailed():
return codexSpawnFailed(_that.message);case MinosError_CodexConnectFailed():
return codexConnectFailed(_that.url,_that.message);case MinosError_CodexProtocolError():
return codexProtocolError(_that.method,_that.message);case MinosError_AgentAlreadyRunning():
return agentAlreadyRunning();case MinosError_AgentNotRunning():
return agentNotRunning();case MinosError_AgentNotSupported():
return agentNotSupported(_that.agent);case MinosError_AgentSessionIdMismatch():
return agentSessionIdMismatch();case MinosError_CfAccessMisconfigured():
return cfAccessMisconfigured(_that.reason);case MinosError_IngestSeqConflict():
return ingestSeqConflict(_that.threadId,_that.seq);case MinosError_ThreadNotFound():
return threadNotFound(_that.threadId);case MinosError_TranslationNotImplemented():
return translationNotImplemented(_that.agent);case MinosError_TranslationFailed():
return translationFailed(_that.agent,_that.message);case MinosError_PairingQrVersionUnsupported():
return pairingQrVersionUnsupported(_that.version);case MinosError_Timeout():
return timeout();case MinosError_NotConnected():
return notConnected();case MinosError_RequestDropped():
return requestDropped();case MinosError_AuthRefreshFailed():
return authRefreshFailed(_that.message);case MinosError_EmailTaken():
return emailTaken();case MinosError_WeakPassword():
return weakPassword();case MinosError_RateLimited():
return rateLimited(_that.retryAfterS);case MinosError_InvalidCredentials():
return invalidCredentials();case MinosError_AgentStartFailed():
return agentStartFailed(_that.reason);case MinosError_PairingTokenExpired():
return pairingTokenExpired();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String addr,  String message)?  bindFailed,TResult? Function( String url,  String message)?  connectFailed,TResult? Function( String reason)?  disconnected,TResult? Function()?  pairingTokenInvalid,TResult? Function( PairingState actual)?  pairingStateMismatch,TResult? Function( String deviceId)?  deviceNotTrusted,TResult? Function( String path,  String message)?  storeIo,TResult? Function( String path,  String message)?  storeCorrupt,TResult? Function( String bin,  BigInt timeoutMs)?  cliProbeTimeout,TResult? Function( String bin,  String message)?  cliProbeFailed,TResult? Function( String method,  String message)?  rpcCallFailed,TResult? Function( String reason)?  unauthorized,TResult? Function( String expected,  String actual)?  connectionStateMismatch,TResult? Function( int version)?  envelopeVersionUnsupported,TResult? Function( String peerDeviceId)?  peerOffline,TResult? Function( String message)?  backendInternal,TResult? Function( String message)?  cfAuthFailed,TResult? Function( String message)?  codexSpawnFailed,TResult? Function( String url,  String message)?  codexConnectFailed,TResult? Function( String method,  String message)?  codexProtocolError,TResult? Function()?  agentAlreadyRunning,TResult? Function()?  agentNotRunning,TResult? Function( AgentName agent)?  agentNotSupported,TResult? Function()?  agentSessionIdMismatch,TResult? Function( String reason)?  cfAccessMisconfigured,TResult? Function( String threadId,  BigInt seq)?  ingestSeqConflict,TResult? Function( String threadId)?  threadNotFound,TResult? Function( AgentName agent)?  translationNotImplemented,TResult? Function( AgentName agent,  String message)?  translationFailed,TResult? Function( int version)?  pairingQrVersionUnsupported,TResult? Function()?  timeout,TResult? Function()?  notConnected,TResult? Function()?  requestDropped,TResult? Function( String message)?  authRefreshFailed,TResult? Function()?  emailTaken,TResult? Function()?  weakPassword,TResult? Function( int retryAfterS)?  rateLimited,TResult? Function()?  invalidCredentials,TResult? Function( String reason)?  agentStartFailed,TResult? Function()?  pairingTokenExpired,}) {final _that = this;
switch (_that) {
case MinosError_BindFailed() when bindFailed != null:
return bindFailed(_that.addr,_that.message);case MinosError_ConnectFailed() when connectFailed != null:
return connectFailed(_that.url,_that.message);case MinosError_Disconnected() when disconnected != null:
return disconnected(_that.reason);case MinosError_PairingTokenInvalid() when pairingTokenInvalid != null:
return pairingTokenInvalid();case MinosError_PairingStateMismatch() when pairingStateMismatch != null:
return pairingStateMismatch(_that.actual);case MinosError_DeviceNotTrusted() when deviceNotTrusted != null:
return deviceNotTrusted(_that.deviceId);case MinosError_StoreIo() when storeIo != null:
return storeIo(_that.path,_that.message);case MinosError_StoreCorrupt() when storeCorrupt != null:
return storeCorrupt(_that.path,_that.message);case MinosError_CliProbeTimeout() when cliProbeTimeout != null:
return cliProbeTimeout(_that.bin,_that.timeoutMs);case MinosError_CliProbeFailed() when cliProbeFailed != null:
return cliProbeFailed(_that.bin,_that.message);case MinosError_RpcCallFailed() when rpcCallFailed != null:
return rpcCallFailed(_that.method,_that.message);case MinosError_Unauthorized() when unauthorized != null:
return unauthorized(_that.reason);case MinosError_ConnectionStateMismatch() when connectionStateMismatch != null:
return connectionStateMismatch(_that.expected,_that.actual);case MinosError_EnvelopeVersionUnsupported() when envelopeVersionUnsupported != null:
return envelopeVersionUnsupported(_that.version);case MinosError_PeerOffline() when peerOffline != null:
return peerOffline(_that.peerDeviceId);case MinosError_BackendInternal() when backendInternal != null:
return backendInternal(_that.message);case MinosError_CfAuthFailed() when cfAuthFailed != null:
return cfAuthFailed(_that.message);case MinosError_CodexSpawnFailed() when codexSpawnFailed != null:
return codexSpawnFailed(_that.message);case MinosError_CodexConnectFailed() when codexConnectFailed != null:
return codexConnectFailed(_that.url,_that.message);case MinosError_CodexProtocolError() when codexProtocolError != null:
return codexProtocolError(_that.method,_that.message);case MinosError_AgentAlreadyRunning() when agentAlreadyRunning != null:
return agentAlreadyRunning();case MinosError_AgentNotRunning() when agentNotRunning != null:
return agentNotRunning();case MinosError_AgentNotSupported() when agentNotSupported != null:
return agentNotSupported(_that.agent);case MinosError_AgentSessionIdMismatch() when agentSessionIdMismatch != null:
return agentSessionIdMismatch();case MinosError_CfAccessMisconfigured() when cfAccessMisconfigured != null:
return cfAccessMisconfigured(_that.reason);case MinosError_IngestSeqConflict() when ingestSeqConflict != null:
return ingestSeqConflict(_that.threadId,_that.seq);case MinosError_ThreadNotFound() when threadNotFound != null:
return threadNotFound(_that.threadId);case MinosError_TranslationNotImplemented() when translationNotImplemented != null:
return translationNotImplemented(_that.agent);case MinosError_TranslationFailed() when translationFailed != null:
return translationFailed(_that.agent,_that.message);case MinosError_PairingQrVersionUnsupported() when pairingQrVersionUnsupported != null:
return pairingQrVersionUnsupported(_that.version);case MinosError_Timeout() when timeout != null:
return timeout();case MinosError_NotConnected() when notConnected != null:
return notConnected();case MinosError_RequestDropped() when requestDropped != null:
return requestDropped();case MinosError_AuthRefreshFailed() when authRefreshFailed != null:
return authRefreshFailed(_that.message);case MinosError_EmailTaken() when emailTaken != null:
return emailTaken();case MinosError_WeakPassword() when weakPassword != null:
return weakPassword();case MinosError_RateLimited() when rateLimited != null:
return rateLimited(_that.retryAfterS);case MinosError_InvalidCredentials() when invalidCredentials != null:
return invalidCredentials();case MinosError_AgentStartFailed() when agentStartFailed != null:
return agentStartFailed(_that.reason);case MinosError_PairingTokenExpired() when pairingTokenExpired != null:
return pairingTokenExpired();case _:
  return null;

}
}

}

/// @nodoc


class MinosError_BindFailed extends MinosError {
  const MinosError_BindFailed({required this.addr, required this.message}): super._();
  

 final  String addr;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_BindFailedCopyWith<MinosError_BindFailed> get copyWith => _$MinosError_BindFailedCopyWithImpl<MinosError_BindFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_BindFailed&&(identical(other.addr, addr) || other.addr == addr)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,addr,message);

@override
String toString() {
  return 'MinosError.bindFailed(addr: $addr, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_BindFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_BindFailedCopyWith(MinosError_BindFailed value, $Res Function(MinosError_BindFailed) _then) = _$MinosError_BindFailedCopyWithImpl;
@useResult
$Res call({
 String addr, String message
});




}
/// @nodoc
class _$MinosError_BindFailedCopyWithImpl<$Res>
    implements $MinosError_BindFailedCopyWith<$Res> {
  _$MinosError_BindFailedCopyWithImpl(this._self, this._then);

  final MinosError_BindFailed _self;
  final $Res Function(MinosError_BindFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? addr = null,Object? message = null,}) {
  return _then(MinosError_BindFailed(
addr: null == addr ? _self.addr : addr // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_ConnectFailed extends MinosError {
  const MinosError_ConnectFailed({required this.url, required this.message}): super._();
  

 final  String url;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_ConnectFailedCopyWith<MinosError_ConnectFailed> get copyWith => _$MinosError_ConnectFailedCopyWithImpl<MinosError_ConnectFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_ConnectFailed&&(identical(other.url, url) || other.url == url)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,url,message);

@override
String toString() {
  return 'MinosError.connectFailed(url: $url, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_ConnectFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_ConnectFailedCopyWith(MinosError_ConnectFailed value, $Res Function(MinosError_ConnectFailed) _then) = _$MinosError_ConnectFailedCopyWithImpl;
@useResult
$Res call({
 String url, String message
});




}
/// @nodoc
class _$MinosError_ConnectFailedCopyWithImpl<$Res>
    implements $MinosError_ConnectFailedCopyWith<$Res> {
  _$MinosError_ConnectFailedCopyWithImpl(this._self, this._then);

  final MinosError_ConnectFailed _self;
  final $Res Function(MinosError_ConnectFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? url = null,Object? message = null,}) {
  return _then(MinosError_ConnectFailed(
url: null == url ? _self.url : url // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_Disconnected extends MinosError {
  const MinosError_Disconnected({required this.reason}): super._();
  

 final  String reason;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_DisconnectedCopyWith<MinosError_Disconnected> get copyWith => _$MinosError_DisconnectedCopyWithImpl<MinosError_Disconnected>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_Disconnected&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode => Object.hash(runtimeType,reason);

@override
String toString() {
  return 'MinosError.disconnected(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $MinosError_DisconnectedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_DisconnectedCopyWith(MinosError_Disconnected value, $Res Function(MinosError_Disconnected) _then) = _$MinosError_DisconnectedCopyWithImpl;
@useResult
$Res call({
 String reason
});




}
/// @nodoc
class _$MinosError_DisconnectedCopyWithImpl<$Res>
    implements $MinosError_DisconnectedCopyWith<$Res> {
  _$MinosError_DisconnectedCopyWithImpl(this._self, this._then);

  final MinosError_Disconnected _self;
  final $Res Function(MinosError_Disconnected) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(MinosError_Disconnected(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_PairingTokenInvalid extends MinosError {
  const MinosError_PairingTokenInvalid(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_PairingTokenInvalid);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.pairingTokenInvalid()';
}


}




/// @nodoc


class MinosError_PairingStateMismatch extends MinosError {
  const MinosError_PairingStateMismatch({required this.actual}): super._();
  

 final  PairingState actual;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_PairingStateMismatchCopyWith<MinosError_PairingStateMismatch> get copyWith => _$MinosError_PairingStateMismatchCopyWithImpl<MinosError_PairingStateMismatch>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_PairingStateMismatch&&(identical(other.actual, actual) || other.actual == actual));
}


@override
int get hashCode => Object.hash(runtimeType,actual);

@override
String toString() {
  return 'MinosError.pairingStateMismatch(actual: $actual)';
}


}

/// @nodoc
abstract mixin class $MinosError_PairingStateMismatchCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_PairingStateMismatchCopyWith(MinosError_PairingStateMismatch value, $Res Function(MinosError_PairingStateMismatch) _then) = _$MinosError_PairingStateMismatchCopyWithImpl;
@useResult
$Res call({
 PairingState actual
});




}
/// @nodoc
class _$MinosError_PairingStateMismatchCopyWithImpl<$Res>
    implements $MinosError_PairingStateMismatchCopyWith<$Res> {
  _$MinosError_PairingStateMismatchCopyWithImpl(this._self, this._then);

  final MinosError_PairingStateMismatch _self;
  final $Res Function(MinosError_PairingStateMismatch) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? actual = null,}) {
  return _then(MinosError_PairingStateMismatch(
actual: null == actual ? _self.actual : actual // ignore: cast_nullable_to_non_nullable
as PairingState,
  ));
}


}

/// @nodoc


class MinosError_DeviceNotTrusted extends MinosError {
  const MinosError_DeviceNotTrusted({required this.deviceId}): super._();
  

 final  String deviceId;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_DeviceNotTrustedCopyWith<MinosError_DeviceNotTrusted> get copyWith => _$MinosError_DeviceNotTrustedCopyWithImpl<MinosError_DeviceNotTrusted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_DeviceNotTrusted&&(identical(other.deviceId, deviceId) || other.deviceId == deviceId));
}


@override
int get hashCode => Object.hash(runtimeType,deviceId);

@override
String toString() {
  return 'MinosError.deviceNotTrusted(deviceId: $deviceId)';
}


}

/// @nodoc
abstract mixin class $MinosError_DeviceNotTrustedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_DeviceNotTrustedCopyWith(MinosError_DeviceNotTrusted value, $Res Function(MinosError_DeviceNotTrusted) _then) = _$MinosError_DeviceNotTrustedCopyWithImpl;
@useResult
$Res call({
 String deviceId
});




}
/// @nodoc
class _$MinosError_DeviceNotTrustedCopyWithImpl<$Res>
    implements $MinosError_DeviceNotTrustedCopyWith<$Res> {
  _$MinosError_DeviceNotTrustedCopyWithImpl(this._self, this._then);

  final MinosError_DeviceNotTrusted _self;
  final $Res Function(MinosError_DeviceNotTrusted) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? deviceId = null,}) {
  return _then(MinosError_DeviceNotTrusted(
deviceId: null == deviceId ? _self.deviceId : deviceId // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_StoreIo extends MinosError {
  const MinosError_StoreIo({required this.path, required this.message}): super._();
  

 final  String path;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_StoreIoCopyWith<MinosError_StoreIo> get copyWith => _$MinosError_StoreIoCopyWithImpl<MinosError_StoreIo>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_StoreIo&&(identical(other.path, path) || other.path == path)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,path,message);

@override
String toString() {
  return 'MinosError.storeIo(path: $path, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_StoreIoCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_StoreIoCopyWith(MinosError_StoreIo value, $Res Function(MinosError_StoreIo) _then) = _$MinosError_StoreIoCopyWithImpl;
@useResult
$Res call({
 String path, String message
});




}
/// @nodoc
class _$MinosError_StoreIoCopyWithImpl<$Res>
    implements $MinosError_StoreIoCopyWith<$Res> {
  _$MinosError_StoreIoCopyWithImpl(this._self, this._then);

  final MinosError_StoreIo _self;
  final $Res Function(MinosError_StoreIo) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? path = null,Object? message = null,}) {
  return _then(MinosError_StoreIo(
path: null == path ? _self.path : path // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_StoreCorrupt extends MinosError {
  const MinosError_StoreCorrupt({required this.path, required this.message}): super._();
  

 final  String path;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_StoreCorruptCopyWith<MinosError_StoreCorrupt> get copyWith => _$MinosError_StoreCorruptCopyWithImpl<MinosError_StoreCorrupt>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_StoreCorrupt&&(identical(other.path, path) || other.path == path)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,path,message);

@override
String toString() {
  return 'MinosError.storeCorrupt(path: $path, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_StoreCorruptCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_StoreCorruptCopyWith(MinosError_StoreCorrupt value, $Res Function(MinosError_StoreCorrupt) _then) = _$MinosError_StoreCorruptCopyWithImpl;
@useResult
$Res call({
 String path, String message
});




}
/// @nodoc
class _$MinosError_StoreCorruptCopyWithImpl<$Res>
    implements $MinosError_StoreCorruptCopyWith<$Res> {
  _$MinosError_StoreCorruptCopyWithImpl(this._self, this._then);

  final MinosError_StoreCorrupt _self;
  final $Res Function(MinosError_StoreCorrupt) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? path = null,Object? message = null,}) {
  return _then(MinosError_StoreCorrupt(
path: null == path ? _self.path : path // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_CliProbeTimeout extends MinosError {
  const MinosError_CliProbeTimeout({required this.bin, required this.timeoutMs}): super._();
  

 final  String bin;
 final  BigInt timeoutMs;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CliProbeTimeoutCopyWith<MinosError_CliProbeTimeout> get copyWith => _$MinosError_CliProbeTimeoutCopyWithImpl<MinosError_CliProbeTimeout>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CliProbeTimeout&&(identical(other.bin, bin) || other.bin == bin)&&(identical(other.timeoutMs, timeoutMs) || other.timeoutMs == timeoutMs));
}


@override
int get hashCode => Object.hash(runtimeType,bin,timeoutMs);

@override
String toString() {
  return 'MinosError.cliProbeTimeout(bin: $bin, timeoutMs: $timeoutMs)';
}


}

/// @nodoc
abstract mixin class $MinosError_CliProbeTimeoutCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CliProbeTimeoutCopyWith(MinosError_CliProbeTimeout value, $Res Function(MinosError_CliProbeTimeout) _then) = _$MinosError_CliProbeTimeoutCopyWithImpl;
@useResult
$Res call({
 String bin, BigInt timeoutMs
});




}
/// @nodoc
class _$MinosError_CliProbeTimeoutCopyWithImpl<$Res>
    implements $MinosError_CliProbeTimeoutCopyWith<$Res> {
  _$MinosError_CliProbeTimeoutCopyWithImpl(this._self, this._then);

  final MinosError_CliProbeTimeout _self;
  final $Res Function(MinosError_CliProbeTimeout) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? bin = null,Object? timeoutMs = null,}) {
  return _then(MinosError_CliProbeTimeout(
bin: null == bin ? _self.bin : bin // ignore: cast_nullable_to_non_nullable
as String,timeoutMs: null == timeoutMs ? _self.timeoutMs : timeoutMs // ignore: cast_nullable_to_non_nullable
as BigInt,
  ));
}


}

/// @nodoc


class MinosError_CliProbeFailed extends MinosError {
  const MinosError_CliProbeFailed({required this.bin, required this.message}): super._();
  

 final  String bin;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CliProbeFailedCopyWith<MinosError_CliProbeFailed> get copyWith => _$MinosError_CliProbeFailedCopyWithImpl<MinosError_CliProbeFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CliProbeFailed&&(identical(other.bin, bin) || other.bin == bin)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,bin,message);

@override
String toString() {
  return 'MinosError.cliProbeFailed(bin: $bin, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_CliProbeFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CliProbeFailedCopyWith(MinosError_CliProbeFailed value, $Res Function(MinosError_CliProbeFailed) _then) = _$MinosError_CliProbeFailedCopyWithImpl;
@useResult
$Res call({
 String bin, String message
});




}
/// @nodoc
class _$MinosError_CliProbeFailedCopyWithImpl<$Res>
    implements $MinosError_CliProbeFailedCopyWith<$Res> {
  _$MinosError_CliProbeFailedCopyWithImpl(this._self, this._then);

  final MinosError_CliProbeFailed _self;
  final $Res Function(MinosError_CliProbeFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? bin = null,Object? message = null,}) {
  return _then(MinosError_CliProbeFailed(
bin: null == bin ? _self.bin : bin // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_RpcCallFailed extends MinosError {
  const MinosError_RpcCallFailed({required this.method, required this.message}): super._();
  

 final  String method;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_RpcCallFailedCopyWith<MinosError_RpcCallFailed> get copyWith => _$MinosError_RpcCallFailedCopyWithImpl<MinosError_RpcCallFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_RpcCallFailed&&(identical(other.method, method) || other.method == method)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,method,message);

@override
String toString() {
  return 'MinosError.rpcCallFailed(method: $method, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_RpcCallFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_RpcCallFailedCopyWith(MinosError_RpcCallFailed value, $Res Function(MinosError_RpcCallFailed) _then) = _$MinosError_RpcCallFailedCopyWithImpl;
@useResult
$Res call({
 String method, String message
});




}
/// @nodoc
class _$MinosError_RpcCallFailedCopyWithImpl<$Res>
    implements $MinosError_RpcCallFailedCopyWith<$Res> {
  _$MinosError_RpcCallFailedCopyWithImpl(this._self, this._then);

  final MinosError_RpcCallFailed _self;
  final $Res Function(MinosError_RpcCallFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? method = null,Object? message = null,}) {
  return _then(MinosError_RpcCallFailed(
method: null == method ? _self.method : method // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_Unauthorized extends MinosError {
  const MinosError_Unauthorized({required this.reason}): super._();
  

 final  String reason;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_UnauthorizedCopyWith<MinosError_Unauthorized> get copyWith => _$MinosError_UnauthorizedCopyWithImpl<MinosError_Unauthorized>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_Unauthorized&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode => Object.hash(runtimeType,reason);

@override
String toString() {
  return 'MinosError.unauthorized(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $MinosError_UnauthorizedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_UnauthorizedCopyWith(MinosError_Unauthorized value, $Res Function(MinosError_Unauthorized) _then) = _$MinosError_UnauthorizedCopyWithImpl;
@useResult
$Res call({
 String reason
});




}
/// @nodoc
class _$MinosError_UnauthorizedCopyWithImpl<$Res>
    implements $MinosError_UnauthorizedCopyWith<$Res> {
  _$MinosError_UnauthorizedCopyWithImpl(this._self, this._then);

  final MinosError_Unauthorized _self;
  final $Res Function(MinosError_Unauthorized) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(MinosError_Unauthorized(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_ConnectionStateMismatch extends MinosError {
  const MinosError_ConnectionStateMismatch({required this.expected, required this.actual}): super._();
  

 final  String expected;
 final  String actual;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_ConnectionStateMismatchCopyWith<MinosError_ConnectionStateMismatch> get copyWith => _$MinosError_ConnectionStateMismatchCopyWithImpl<MinosError_ConnectionStateMismatch>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_ConnectionStateMismatch&&(identical(other.expected, expected) || other.expected == expected)&&(identical(other.actual, actual) || other.actual == actual));
}


@override
int get hashCode => Object.hash(runtimeType,expected,actual);

@override
String toString() {
  return 'MinosError.connectionStateMismatch(expected: $expected, actual: $actual)';
}


}

/// @nodoc
abstract mixin class $MinosError_ConnectionStateMismatchCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_ConnectionStateMismatchCopyWith(MinosError_ConnectionStateMismatch value, $Res Function(MinosError_ConnectionStateMismatch) _then) = _$MinosError_ConnectionStateMismatchCopyWithImpl;
@useResult
$Res call({
 String expected, String actual
});




}
/// @nodoc
class _$MinosError_ConnectionStateMismatchCopyWithImpl<$Res>
    implements $MinosError_ConnectionStateMismatchCopyWith<$Res> {
  _$MinosError_ConnectionStateMismatchCopyWithImpl(this._self, this._then);

  final MinosError_ConnectionStateMismatch _self;
  final $Res Function(MinosError_ConnectionStateMismatch) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? expected = null,Object? actual = null,}) {
  return _then(MinosError_ConnectionStateMismatch(
expected: null == expected ? _self.expected : expected // ignore: cast_nullable_to_non_nullable
as String,actual: null == actual ? _self.actual : actual // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_EnvelopeVersionUnsupported extends MinosError {
  const MinosError_EnvelopeVersionUnsupported({required this.version}): super._();
  

 final  int version;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_EnvelopeVersionUnsupportedCopyWith<MinosError_EnvelopeVersionUnsupported> get copyWith => _$MinosError_EnvelopeVersionUnsupportedCopyWithImpl<MinosError_EnvelopeVersionUnsupported>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_EnvelopeVersionUnsupported&&(identical(other.version, version) || other.version == version));
}


@override
int get hashCode => Object.hash(runtimeType,version);

@override
String toString() {
  return 'MinosError.envelopeVersionUnsupported(version: $version)';
}


}

/// @nodoc
abstract mixin class $MinosError_EnvelopeVersionUnsupportedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_EnvelopeVersionUnsupportedCopyWith(MinosError_EnvelopeVersionUnsupported value, $Res Function(MinosError_EnvelopeVersionUnsupported) _then) = _$MinosError_EnvelopeVersionUnsupportedCopyWithImpl;
@useResult
$Res call({
 int version
});




}
/// @nodoc
class _$MinosError_EnvelopeVersionUnsupportedCopyWithImpl<$Res>
    implements $MinosError_EnvelopeVersionUnsupportedCopyWith<$Res> {
  _$MinosError_EnvelopeVersionUnsupportedCopyWithImpl(this._self, this._then);

  final MinosError_EnvelopeVersionUnsupported _self;
  final $Res Function(MinosError_EnvelopeVersionUnsupported) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? version = null,}) {
  return _then(MinosError_EnvelopeVersionUnsupported(
version: null == version ? _self.version : version // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class MinosError_PeerOffline extends MinosError {
  const MinosError_PeerOffline({required this.peerDeviceId}): super._();
  

 final  String peerDeviceId;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_PeerOfflineCopyWith<MinosError_PeerOffline> get copyWith => _$MinosError_PeerOfflineCopyWithImpl<MinosError_PeerOffline>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_PeerOffline&&(identical(other.peerDeviceId, peerDeviceId) || other.peerDeviceId == peerDeviceId));
}


@override
int get hashCode => Object.hash(runtimeType,peerDeviceId);

@override
String toString() {
  return 'MinosError.peerOffline(peerDeviceId: $peerDeviceId)';
}


}

/// @nodoc
abstract mixin class $MinosError_PeerOfflineCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_PeerOfflineCopyWith(MinosError_PeerOffline value, $Res Function(MinosError_PeerOffline) _then) = _$MinosError_PeerOfflineCopyWithImpl;
@useResult
$Res call({
 String peerDeviceId
});




}
/// @nodoc
class _$MinosError_PeerOfflineCopyWithImpl<$Res>
    implements $MinosError_PeerOfflineCopyWith<$Res> {
  _$MinosError_PeerOfflineCopyWithImpl(this._self, this._then);

  final MinosError_PeerOffline _self;
  final $Res Function(MinosError_PeerOffline) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? peerDeviceId = null,}) {
  return _then(MinosError_PeerOffline(
peerDeviceId: null == peerDeviceId ? _self.peerDeviceId : peerDeviceId // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_BackendInternal extends MinosError {
  const MinosError_BackendInternal({required this.message}): super._();
  

 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_BackendInternalCopyWith<MinosError_BackendInternal> get copyWith => _$MinosError_BackendInternalCopyWithImpl<MinosError_BackendInternal>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_BackendInternal&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,message);

@override
String toString() {
  return 'MinosError.backendInternal(message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_BackendInternalCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_BackendInternalCopyWith(MinosError_BackendInternal value, $Res Function(MinosError_BackendInternal) _then) = _$MinosError_BackendInternalCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$MinosError_BackendInternalCopyWithImpl<$Res>
    implements $MinosError_BackendInternalCopyWith<$Res> {
  _$MinosError_BackendInternalCopyWithImpl(this._self, this._then);

  final MinosError_BackendInternal _self;
  final $Res Function(MinosError_BackendInternal) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(MinosError_BackendInternal(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_CfAuthFailed extends MinosError {
  const MinosError_CfAuthFailed({required this.message}): super._();
  

 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CfAuthFailedCopyWith<MinosError_CfAuthFailed> get copyWith => _$MinosError_CfAuthFailedCopyWithImpl<MinosError_CfAuthFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CfAuthFailed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,message);

@override
String toString() {
  return 'MinosError.cfAuthFailed(message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_CfAuthFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CfAuthFailedCopyWith(MinosError_CfAuthFailed value, $Res Function(MinosError_CfAuthFailed) _then) = _$MinosError_CfAuthFailedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$MinosError_CfAuthFailedCopyWithImpl<$Res>
    implements $MinosError_CfAuthFailedCopyWith<$Res> {
  _$MinosError_CfAuthFailedCopyWithImpl(this._self, this._then);

  final MinosError_CfAuthFailed _self;
  final $Res Function(MinosError_CfAuthFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(MinosError_CfAuthFailed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_CodexSpawnFailed extends MinosError {
  const MinosError_CodexSpawnFailed({required this.message}): super._();
  

 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CodexSpawnFailedCopyWith<MinosError_CodexSpawnFailed> get copyWith => _$MinosError_CodexSpawnFailedCopyWithImpl<MinosError_CodexSpawnFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CodexSpawnFailed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,message);

@override
String toString() {
  return 'MinosError.codexSpawnFailed(message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_CodexSpawnFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CodexSpawnFailedCopyWith(MinosError_CodexSpawnFailed value, $Res Function(MinosError_CodexSpawnFailed) _then) = _$MinosError_CodexSpawnFailedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$MinosError_CodexSpawnFailedCopyWithImpl<$Res>
    implements $MinosError_CodexSpawnFailedCopyWith<$Res> {
  _$MinosError_CodexSpawnFailedCopyWithImpl(this._self, this._then);

  final MinosError_CodexSpawnFailed _self;
  final $Res Function(MinosError_CodexSpawnFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(MinosError_CodexSpawnFailed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_CodexConnectFailed extends MinosError {
  const MinosError_CodexConnectFailed({required this.url, required this.message}): super._();
  

 final  String url;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CodexConnectFailedCopyWith<MinosError_CodexConnectFailed> get copyWith => _$MinosError_CodexConnectFailedCopyWithImpl<MinosError_CodexConnectFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CodexConnectFailed&&(identical(other.url, url) || other.url == url)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,url,message);

@override
String toString() {
  return 'MinosError.codexConnectFailed(url: $url, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_CodexConnectFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CodexConnectFailedCopyWith(MinosError_CodexConnectFailed value, $Res Function(MinosError_CodexConnectFailed) _then) = _$MinosError_CodexConnectFailedCopyWithImpl;
@useResult
$Res call({
 String url, String message
});




}
/// @nodoc
class _$MinosError_CodexConnectFailedCopyWithImpl<$Res>
    implements $MinosError_CodexConnectFailedCopyWith<$Res> {
  _$MinosError_CodexConnectFailedCopyWithImpl(this._self, this._then);

  final MinosError_CodexConnectFailed _self;
  final $Res Function(MinosError_CodexConnectFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? url = null,Object? message = null,}) {
  return _then(MinosError_CodexConnectFailed(
url: null == url ? _self.url : url // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_CodexProtocolError extends MinosError {
  const MinosError_CodexProtocolError({required this.method, required this.message}): super._();
  

 final  String method;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CodexProtocolErrorCopyWith<MinosError_CodexProtocolError> get copyWith => _$MinosError_CodexProtocolErrorCopyWithImpl<MinosError_CodexProtocolError>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CodexProtocolError&&(identical(other.method, method) || other.method == method)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,method,message);

@override
String toString() {
  return 'MinosError.codexProtocolError(method: $method, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_CodexProtocolErrorCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CodexProtocolErrorCopyWith(MinosError_CodexProtocolError value, $Res Function(MinosError_CodexProtocolError) _then) = _$MinosError_CodexProtocolErrorCopyWithImpl;
@useResult
$Res call({
 String method, String message
});




}
/// @nodoc
class _$MinosError_CodexProtocolErrorCopyWithImpl<$Res>
    implements $MinosError_CodexProtocolErrorCopyWith<$Res> {
  _$MinosError_CodexProtocolErrorCopyWithImpl(this._self, this._then);

  final MinosError_CodexProtocolError _self;
  final $Res Function(MinosError_CodexProtocolError) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? method = null,Object? message = null,}) {
  return _then(MinosError_CodexProtocolError(
method: null == method ? _self.method : method // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_AgentAlreadyRunning extends MinosError {
  const MinosError_AgentAlreadyRunning(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AgentAlreadyRunning);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.agentAlreadyRunning()';
}


}




/// @nodoc


class MinosError_AgentNotRunning extends MinosError {
  const MinosError_AgentNotRunning(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AgentNotRunning);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.agentNotRunning()';
}


}




/// @nodoc


class MinosError_AgentNotSupported extends MinosError {
  const MinosError_AgentNotSupported({required this.agent}): super._();
  

 final  AgentName agent;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_AgentNotSupportedCopyWith<MinosError_AgentNotSupported> get copyWith => _$MinosError_AgentNotSupportedCopyWithImpl<MinosError_AgentNotSupported>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AgentNotSupported&&(identical(other.agent, agent) || other.agent == agent));
}


@override
int get hashCode => Object.hash(runtimeType,agent);

@override
String toString() {
  return 'MinosError.agentNotSupported(agent: $agent)';
}


}

/// @nodoc
abstract mixin class $MinosError_AgentNotSupportedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_AgentNotSupportedCopyWith(MinosError_AgentNotSupported value, $Res Function(MinosError_AgentNotSupported) _then) = _$MinosError_AgentNotSupportedCopyWithImpl;
@useResult
$Res call({
 AgentName agent
});




}
/// @nodoc
class _$MinosError_AgentNotSupportedCopyWithImpl<$Res>
    implements $MinosError_AgentNotSupportedCopyWith<$Res> {
  _$MinosError_AgentNotSupportedCopyWithImpl(this._self, this._then);

  final MinosError_AgentNotSupported _self;
  final $Res Function(MinosError_AgentNotSupported) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? agent = null,}) {
  return _then(MinosError_AgentNotSupported(
agent: null == agent ? _self.agent : agent // ignore: cast_nullable_to_non_nullable
as AgentName,
  ));
}


}

/// @nodoc


class MinosError_AgentSessionIdMismatch extends MinosError {
  const MinosError_AgentSessionIdMismatch(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AgentSessionIdMismatch);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.agentSessionIdMismatch()';
}


}




/// @nodoc


class MinosError_CfAccessMisconfigured extends MinosError {
  const MinosError_CfAccessMisconfigured({required this.reason}): super._();
  

 final  String reason;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_CfAccessMisconfiguredCopyWith<MinosError_CfAccessMisconfigured> get copyWith => _$MinosError_CfAccessMisconfiguredCopyWithImpl<MinosError_CfAccessMisconfigured>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_CfAccessMisconfigured&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode => Object.hash(runtimeType,reason);

@override
String toString() {
  return 'MinosError.cfAccessMisconfigured(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $MinosError_CfAccessMisconfiguredCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_CfAccessMisconfiguredCopyWith(MinosError_CfAccessMisconfigured value, $Res Function(MinosError_CfAccessMisconfigured) _then) = _$MinosError_CfAccessMisconfiguredCopyWithImpl;
@useResult
$Res call({
 String reason
});




}
/// @nodoc
class _$MinosError_CfAccessMisconfiguredCopyWithImpl<$Res>
    implements $MinosError_CfAccessMisconfiguredCopyWith<$Res> {
  _$MinosError_CfAccessMisconfiguredCopyWithImpl(this._self, this._then);

  final MinosError_CfAccessMisconfigured _self;
  final $Res Function(MinosError_CfAccessMisconfigured) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(MinosError_CfAccessMisconfigured(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_IngestSeqConflict extends MinosError {
  const MinosError_IngestSeqConflict({required this.threadId, required this.seq}): super._();
  

 final  String threadId;
 final  BigInt seq;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_IngestSeqConflictCopyWith<MinosError_IngestSeqConflict> get copyWith => _$MinosError_IngestSeqConflictCopyWithImpl<MinosError_IngestSeqConflict>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_IngestSeqConflict&&(identical(other.threadId, threadId) || other.threadId == threadId)&&(identical(other.seq, seq) || other.seq == seq));
}


@override
int get hashCode => Object.hash(runtimeType,threadId,seq);

@override
String toString() {
  return 'MinosError.ingestSeqConflict(threadId: $threadId, seq: $seq)';
}


}

/// @nodoc
abstract mixin class $MinosError_IngestSeqConflictCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_IngestSeqConflictCopyWith(MinosError_IngestSeqConflict value, $Res Function(MinosError_IngestSeqConflict) _then) = _$MinosError_IngestSeqConflictCopyWithImpl;
@useResult
$Res call({
 String threadId, BigInt seq
});




}
/// @nodoc
class _$MinosError_IngestSeqConflictCopyWithImpl<$Res>
    implements $MinosError_IngestSeqConflictCopyWith<$Res> {
  _$MinosError_IngestSeqConflictCopyWithImpl(this._self, this._then);

  final MinosError_IngestSeqConflict _self;
  final $Res Function(MinosError_IngestSeqConflict) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? threadId = null,Object? seq = null,}) {
  return _then(MinosError_IngestSeqConflict(
threadId: null == threadId ? _self.threadId : threadId // ignore: cast_nullable_to_non_nullable
as String,seq: null == seq ? _self.seq : seq // ignore: cast_nullable_to_non_nullable
as BigInt,
  ));
}


}

/// @nodoc


class MinosError_ThreadNotFound extends MinosError {
  const MinosError_ThreadNotFound({required this.threadId}): super._();
  

 final  String threadId;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_ThreadNotFoundCopyWith<MinosError_ThreadNotFound> get copyWith => _$MinosError_ThreadNotFoundCopyWithImpl<MinosError_ThreadNotFound>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_ThreadNotFound&&(identical(other.threadId, threadId) || other.threadId == threadId));
}


@override
int get hashCode => Object.hash(runtimeType,threadId);

@override
String toString() {
  return 'MinosError.threadNotFound(threadId: $threadId)';
}


}

/// @nodoc
abstract mixin class $MinosError_ThreadNotFoundCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_ThreadNotFoundCopyWith(MinosError_ThreadNotFound value, $Res Function(MinosError_ThreadNotFound) _then) = _$MinosError_ThreadNotFoundCopyWithImpl;
@useResult
$Res call({
 String threadId
});




}
/// @nodoc
class _$MinosError_ThreadNotFoundCopyWithImpl<$Res>
    implements $MinosError_ThreadNotFoundCopyWith<$Res> {
  _$MinosError_ThreadNotFoundCopyWithImpl(this._self, this._then);

  final MinosError_ThreadNotFound _self;
  final $Res Function(MinosError_ThreadNotFound) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? threadId = null,}) {
  return _then(MinosError_ThreadNotFound(
threadId: null == threadId ? _self.threadId : threadId // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_TranslationNotImplemented extends MinosError {
  const MinosError_TranslationNotImplemented({required this.agent}): super._();
  

 final  AgentName agent;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_TranslationNotImplementedCopyWith<MinosError_TranslationNotImplemented> get copyWith => _$MinosError_TranslationNotImplementedCopyWithImpl<MinosError_TranslationNotImplemented>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_TranslationNotImplemented&&(identical(other.agent, agent) || other.agent == agent));
}


@override
int get hashCode => Object.hash(runtimeType,agent);

@override
String toString() {
  return 'MinosError.translationNotImplemented(agent: $agent)';
}


}

/// @nodoc
abstract mixin class $MinosError_TranslationNotImplementedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_TranslationNotImplementedCopyWith(MinosError_TranslationNotImplemented value, $Res Function(MinosError_TranslationNotImplemented) _then) = _$MinosError_TranslationNotImplementedCopyWithImpl;
@useResult
$Res call({
 AgentName agent
});




}
/// @nodoc
class _$MinosError_TranslationNotImplementedCopyWithImpl<$Res>
    implements $MinosError_TranslationNotImplementedCopyWith<$Res> {
  _$MinosError_TranslationNotImplementedCopyWithImpl(this._self, this._then);

  final MinosError_TranslationNotImplemented _self;
  final $Res Function(MinosError_TranslationNotImplemented) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? agent = null,}) {
  return _then(MinosError_TranslationNotImplemented(
agent: null == agent ? _self.agent : agent // ignore: cast_nullable_to_non_nullable
as AgentName,
  ));
}


}

/// @nodoc


class MinosError_TranslationFailed extends MinosError {
  const MinosError_TranslationFailed({required this.agent, required this.message}): super._();
  

 final  AgentName agent;
 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_TranslationFailedCopyWith<MinosError_TranslationFailed> get copyWith => _$MinosError_TranslationFailedCopyWithImpl<MinosError_TranslationFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_TranslationFailed&&(identical(other.agent, agent) || other.agent == agent)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,agent,message);

@override
String toString() {
  return 'MinosError.translationFailed(agent: $agent, message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_TranslationFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_TranslationFailedCopyWith(MinosError_TranslationFailed value, $Res Function(MinosError_TranslationFailed) _then) = _$MinosError_TranslationFailedCopyWithImpl;
@useResult
$Res call({
 AgentName agent, String message
});




}
/// @nodoc
class _$MinosError_TranslationFailedCopyWithImpl<$Res>
    implements $MinosError_TranslationFailedCopyWith<$Res> {
  _$MinosError_TranslationFailedCopyWithImpl(this._self, this._then);

  final MinosError_TranslationFailed _self;
  final $Res Function(MinosError_TranslationFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? agent = null,Object? message = null,}) {
  return _then(MinosError_TranslationFailed(
agent: null == agent ? _self.agent : agent // ignore: cast_nullable_to_non_nullable
as AgentName,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_PairingQrVersionUnsupported extends MinosError {
  const MinosError_PairingQrVersionUnsupported({required this.version}): super._();
  

 final  int version;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_PairingQrVersionUnsupportedCopyWith<MinosError_PairingQrVersionUnsupported> get copyWith => _$MinosError_PairingQrVersionUnsupportedCopyWithImpl<MinosError_PairingQrVersionUnsupported>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_PairingQrVersionUnsupported&&(identical(other.version, version) || other.version == version));
}


@override
int get hashCode => Object.hash(runtimeType,version);

@override
String toString() {
  return 'MinosError.pairingQrVersionUnsupported(version: $version)';
}


}

/// @nodoc
abstract mixin class $MinosError_PairingQrVersionUnsupportedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_PairingQrVersionUnsupportedCopyWith(MinosError_PairingQrVersionUnsupported value, $Res Function(MinosError_PairingQrVersionUnsupported) _then) = _$MinosError_PairingQrVersionUnsupportedCopyWithImpl;
@useResult
$Res call({
 int version
});




}
/// @nodoc
class _$MinosError_PairingQrVersionUnsupportedCopyWithImpl<$Res>
    implements $MinosError_PairingQrVersionUnsupportedCopyWith<$Res> {
  _$MinosError_PairingQrVersionUnsupportedCopyWithImpl(this._self, this._then);

  final MinosError_PairingQrVersionUnsupported _self;
  final $Res Function(MinosError_PairingQrVersionUnsupported) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? version = null,}) {
  return _then(MinosError_PairingQrVersionUnsupported(
version: null == version ? _self.version : version // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class MinosError_Timeout extends MinosError {
  const MinosError_Timeout(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_Timeout);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.timeout()';
}


}




/// @nodoc


class MinosError_NotConnected extends MinosError {
  const MinosError_NotConnected(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_NotConnected);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.notConnected()';
}


}




/// @nodoc


class MinosError_RequestDropped extends MinosError {
  const MinosError_RequestDropped(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_RequestDropped);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.requestDropped()';
}


}




/// @nodoc


class MinosError_AuthRefreshFailed extends MinosError {
  const MinosError_AuthRefreshFailed({required this.message}): super._();
  

 final  String message;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_AuthRefreshFailedCopyWith<MinosError_AuthRefreshFailed> get copyWith => _$MinosError_AuthRefreshFailedCopyWithImpl<MinosError_AuthRefreshFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AuthRefreshFailed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,message);

@override
String toString() {
  return 'MinosError.authRefreshFailed(message: $message)';
}


}

/// @nodoc
abstract mixin class $MinosError_AuthRefreshFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_AuthRefreshFailedCopyWith(MinosError_AuthRefreshFailed value, $Res Function(MinosError_AuthRefreshFailed) _then) = _$MinosError_AuthRefreshFailedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$MinosError_AuthRefreshFailedCopyWithImpl<$Res>
    implements $MinosError_AuthRefreshFailedCopyWith<$Res> {
  _$MinosError_AuthRefreshFailedCopyWithImpl(this._self, this._then);

  final MinosError_AuthRefreshFailed _self;
  final $Res Function(MinosError_AuthRefreshFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(MinosError_AuthRefreshFailed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_EmailTaken extends MinosError {
  const MinosError_EmailTaken(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_EmailTaken);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.emailTaken()';
}


}




/// @nodoc


class MinosError_WeakPassword extends MinosError {
  const MinosError_WeakPassword(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_WeakPassword);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.weakPassword()';
}


}




/// @nodoc


class MinosError_RateLimited extends MinosError {
  const MinosError_RateLimited({required this.retryAfterS}): super._();
  

 final  int retryAfterS;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_RateLimitedCopyWith<MinosError_RateLimited> get copyWith => _$MinosError_RateLimitedCopyWithImpl<MinosError_RateLimited>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_RateLimited&&(identical(other.retryAfterS, retryAfterS) || other.retryAfterS == retryAfterS));
}


@override
int get hashCode => Object.hash(runtimeType,retryAfterS);

@override
String toString() {
  return 'MinosError.rateLimited(retryAfterS: $retryAfterS)';
}


}

/// @nodoc
abstract mixin class $MinosError_RateLimitedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_RateLimitedCopyWith(MinosError_RateLimited value, $Res Function(MinosError_RateLimited) _then) = _$MinosError_RateLimitedCopyWithImpl;
@useResult
$Res call({
 int retryAfterS
});




}
/// @nodoc
class _$MinosError_RateLimitedCopyWithImpl<$Res>
    implements $MinosError_RateLimitedCopyWith<$Res> {
  _$MinosError_RateLimitedCopyWithImpl(this._self, this._then);

  final MinosError_RateLimited _self;
  final $Res Function(MinosError_RateLimited) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? retryAfterS = null,}) {
  return _then(MinosError_RateLimited(
retryAfterS: null == retryAfterS ? _self.retryAfterS : retryAfterS // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class MinosError_InvalidCredentials extends MinosError {
  const MinosError_InvalidCredentials(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_InvalidCredentials);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.invalidCredentials()';
}


}




/// @nodoc


class MinosError_AgentStartFailed extends MinosError {
  const MinosError_AgentStartFailed({required this.reason}): super._();
  

 final  String reason;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MinosError_AgentStartFailedCopyWith<MinosError_AgentStartFailed> get copyWith => _$MinosError_AgentStartFailedCopyWithImpl<MinosError_AgentStartFailed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_AgentStartFailed&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode => Object.hash(runtimeType,reason);

@override
String toString() {
  return 'MinosError.agentStartFailed(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $MinosError_AgentStartFailedCopyWith<$Res> implements $MinosErrorCopyWith<$Res> {
  factory $MinosError_AgentStartFailedCopyWith(MinosError_AgentStartFailed value, $Res Function(MinosError_AgentStartFailed) _then) = _$MinosError_AgentStartFailedCopyWithImpl;
@useResult
$Res call({
 String reason
});




}
/// @nodoc
class _$MinosError_AgentStartFailedCopyWithImpl<$Res>
    implements $MinosError_AgentStartFailedCopyWith<$Res> {
  _$MinosError_AgentStartFailedCopyWithImpl(this._self, this._then);

  final MinosError_AgentStartFailed _self;
  final $Res Function(MinosError_AgentStartFailed) _then;

/// Create a copy of MinosError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(MinosError_AgentStartFailed(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class MinosError_PairingTokenExpired extends MinosError {
  const MinosError_PairingTokenExpired(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is MinosError_PairingTokenExpired);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'MinosError.pairingTokenExpired()';
}


}




/// @nodoc
mixin _$ThreadEndReason {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ThreadEndReason()';
}


}

/// @nodoc
class $ThreadEndReasonCopyWith<$Res>  {
$ThreadEndReasonCopyWith(ThreadEndReason _, $Res Function(ThreadEndReason) __);
}


/// Adds pattern-matching-related methods to [ThreadEndReason].
extension ThreadEndReasonPatterns on ThreadEndReason {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ThreadEndReason_UserStopped value)?  userStopped,TResult Function( ThreadEndReason_AgentDone value)?  agentDone,TResult Function( ThreadEndReason_Crashed value)?  crashed,TResult Function( ThreadEndReason_Timeout value)?  timeout,TResult Function( ThreadEndReason_HostDisconnected value)?  hostDisconnected,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped() when userStopped != null:
return userStopped(_that);case ThreadEndReason_AgentDone() when agentDone != null:
return agentDone(_that);case ThreadEndReason_Crashed() when crashed != null:
return crashed(_that);case ThreadEndReason_Timeout() when timeout != null:
return timeout(_that);case ThreadEndReason_HostDisconnected() when hostDisconnected != null:
return hostDisconnected(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ThreadEndReason_UserStopped value)  userStopped,required TResult Function( ThreadEndReason_AgentDone value)  agentDone,required TResult Function( ThreadEndReason_Crashed value)  crashed,required TResult Function( ThreadEndReason_Timeout value)  timeout,required TResult Function( ThreadEndReason_HostDisconnected value)  hostDisconnected,}){
final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped():
return userStopped(_that);case ThreadEndReason_AgentDone():
return agentDone(_that);case ThreadEndReason_Crashed():
return crashed(_that);case ThreadEndReason_Timeout():
return timeout(_that);case ThreadEndReason_HostDisconnected():
return hostDisconnected(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ThreadEndReason_UserStopped value)?  userStopped,TResult? Function( ThreadEndReason_AgentDone value)?  agentDone,TResult? Function( ThreadEndReason_Crashed value)?  crashed,TResult? Function( ThreadEndReason_Timeout value)?  timeout,TResult? Function( ThreadEndReason_HostDisconnected value)?  hostDisconnected,}){
final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped() when userStopped != null:
return userStopped(_that);case ThreadEndReason_AgentDone() when agentDone != null:
return agentDone(_that);case ThreadEndReason_Crashed() when crashed != null:
return crashed(_that);case ThreadEndReason_Timeout() when timeout != null:
return timeout(_that);case ThreadEndReason_HostDisconnected() when hostDisconnected != null:
return hostDisconnected(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  userStopped,TResult Function()?  agentDone,TResult Function( String message)?  crashed,TResult Function()?  timeout,TResult Function()?  hostDisconnected,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped() when userStopped != null:
return userStopped();case ThreadEndReason_AgentDone() when agentDone != null:
return agentDone();case ThreadEndReason_Crashed() when crashed != null:
return crashed(_that.message);case ThreadEndReason_Timeout() when timeout != null:
return timeout();case ThreadEndReason_HostDisconnected() when hostDisconnected != null:
return hostDisconnected();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  userStopped,required TResult Function()  agentDone,required TResult Function( String message)  crashed,required TResult Function()  timeout,required TResult Function()  hostDisconnected,}) {final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped():
return userStopped();case ThreadEndReason_AgentDone():
return agentDone();case ThreadEndReason_Crashed():
return crashed(_that.message);case ThreadEndReason_Timeout():
return timeout();case ThreadEndReason_HostDisconnected():
return hostDisconnected();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  userStopped,TResult? Function()?  agentDone,TResult? Function( String message)?  crashed,TResult? Function()?  timeout,TResult? Function()?  hostDisconnected,}) {final _that = this;
switch (_that) {
case ThreadEndReason_UserStopped() when userStopped != null:
return userStopped();case ThreadEndReason_AgentDone() when agentDone != null:
return agentDone();case ThreadEndReason_Crashed() when crashed != null:
return crashed(_that.message);case ThreadEndReason_Timeout() when timeout != null:
return timeout();case ThreadEndReason_HostDisconnected() when hostDisconnected != null:
return hostDisconnected();case _:
  return null;

}
}

}

/// @nodoc


class ThreadEndReason_UserStopped extends ThreadEndReason {
  const ThreadEndReason_UserStopped(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason_UserStopped);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ThreadEndReason.userStopped()';
}


}




/// @nodoc


class ThreadEndReason_AgentDone extends ThreadEndReason {
  const ThreadEndReason_AgentDone(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason_AgentDone);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ThreadEndReason.agentDone()';
}


}




/// @nodoc


class ThreadEndReason_Crashed extends ThreadEndReason {
  const ThreadEndReason_Crashed({required this.message}): super._();
  

 final  String message;

/// Create a copy of ThreadEndReason
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ThreadEndReason_CrashedCopyWith<ThreadEndReason_Crashed> get copyWith => _$ThreadEndReason_CrashedCopyWithImpl<ThreadEndReason_Crashed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason_Crashed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode => Object.hash(runtimeType,message);

@override
String toString() {
  return 'ThreadEndReason.crashed(message: $message)';
}


}

/// @nodoc
abstract mixin class $ThreadEndReason_CrashedCopyWith<$Res> implements $ThreadEndReasonCopyWith<$Res> {
  factory $ThreadEndReason_CrashedCopyWith(ThreadEndReason_Crashed value, $Res Function(ThreadEndReason_Crashed) _then) = _$ThreadEndReason_CrashedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$ThreadEndReason_CrashedCopyWithImpl<$Res>
    implements $ThreadEndReason_CrashedCopyWith<$Res> {
  _$ThreadEndReason_CrashedCopyWithImpl(this._self, this._then);

  final ThreadEndReason_Crashed _self;
  final $Res Function(ThreadEndReason_Crashed) _then;

/// Create a copy of ThreadEndReason
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(ThreadEndReason_Crashed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class ThreadEndReason_Timeout extends ThreadEndReason {
  const ThreadEndReason_Timeout(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason_Timeout);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ThreadEndReason.timeout()';
}


}




/// @nodoc


class ThreadEndReason_HostDisconnected extends ThreadEndReason {
  const ThreadEndReason_HostDisconnected(): super._();
  






@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ThreadEndReason_HostDisconnected);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'ThreadEndReason.hostDisconnected()';
}


}




/// @nodoc
mixin _$UiEventMessage {





@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
  return 'UiEventMessage()';
}


}

/// @nodoc
class $UiEventMessageCopyWith<$Res>  {
$UiEventMessageCopyWith(UiEventMessage _, $Res Function(UiEventMessage) __);
}


/// Adds pattern-matching-related methods to [UiEventMessage].
extension UiEventMessagePatterns on UiEventMessage {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( UiEventMessage_ThreadOpened value)?  threadOpened,TResult Function( UiEventMessage_ThreadTitleUpdated value)?  threadTitleUpdated,TResult Function( UiEventMessage_ThreadClosed value)?  threadClosed,TResult Function( UiEventMessage_MessageStarted value)?  messageStarted,TResult Function( UiEventMessage_MessageCompleted value)?  messageCompleted,TResult Function( UiEventMessage_TextDelta value)?  textDelta,TResult Function( UiEventMessage_ReasoningDelta value)?  reasoningDelta,TResult Function( UiEventMessage_ToolCallPlaced value)?  toolCallPlaced,TResult Function( UiEventMessage_ToolCallCompleted value)?  toolCallCompleted,TResult Function( UiEventMessage_Error value)?  error,TResult Function( UiEventMessage_Raw value)?  raw,required TResult orElse(),}){
final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened() when threadOpened != null:
return threadOpened(_that);case UiEventMessage_ThreadTitleUpdated() when threadTitleUpdated != null:
return threadTitleUpdated(_that);case UiEventMessage_ThreadClosed() when threadClosed != null:
return threadClosed(_that);case UiEventMessage_MessageStarted() when messageStarted != null:
return messageStarted(_that);case UiEventMessage_MessageCompleted() when messageCompleted != null:
return messageCompleted(_that);case UiEventMessage_TextDelta() when textDelta != null:
return textDelta(_that);case UiEventMessage_ReasoningDelta() when reasoningDelta != null:
return reasoningDelta(_that);case UiEventMessage_ToolCallPlaced() when toolCallPlaced != null:
return toolCallPlaced(_that);case UiEventMessage_ToolCallCompleted() when toolCallCompleted != null:
return toolCallCompleted(_that);case UiEventMessage_Error() when error != null:
return error(_that);case UiEventMessage_Raw() when raw != null:
return raw(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( UiEventMessage_ThreadOpened value)  threadOpened,required TResult Function( UiEventMessage_ThreadTitleUpdated value)  threadTitleUpdated,required TResult Function( UiEventMessage_ThreadClosed value)  threadClosed,required TResult Function( UiEventMessage_MessageStarted value)  messageStarted,required TResult Function( UiEventMessage_MessageCompleted value)  messageCompleted,required TResult Function( UiEventMessage_TextDelta value)  textDelta,required TResult Function( UiEventMessage_ReasoningDelta value)  reasoningDelta,required TResult Function( UiEventMessage_ToolCallPlaced value)  toolCallPlaced,required TResult Function( UiEventMessage_ToolCallCompleted value)  toolCallCompleted,required TResult Function( UiEventMessage_Error value)  error,required TResult Function( UiEventMessage_Raw value)  raw,}){
final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened():
return threadOpened(_that);case UiEventMessage_ThreadTitleUpdated():
return threadTitleUpdated(_that);case UiEventMessage_ThreadClosed():
return threadClosed(_that);case UiEventMessage_MessageStarted():
return messageStarted(_that);case UiEventMessage_MessageCompleted():
return messageCompleted(_that);case UiEventMessage_TextDelta():
return textDelta(_that);case UiEventMessage_ReasoningDelta():
return reasoningDelta(_that);case UiEventMessage_ToolCallPlaced():
return toolCallPlaced(_that);case UiEventMessage_ToolCallCompleted():
return toolCallCompleted(_that);case UiEventMessage_Error():
return error(_that);case UiEventMessage_Raw():
return raw(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( UiEventMessage_ThreadOpened value)?  threadOpened,TResult? Function( UiEventMessage_ThreadTitleUpdated value)?  threadTitleUpdated,TResult? Function( UiEventMessage_ThreadClosed value)?  threadClosed,TResult? Function( UiEventMessage_MessageStarted value)?  messageStarted,TResult? Function( UiEventMessage_MessageCompleted value)?  messageCompleted,TResult? Function( UiEventMessage_TextDelta value)?  textDelta,TResult? Function( UiEventMessage_ReasoningDelta value)?  reasoningDelta,TResult? Function( UiEventMessage_ToolCallPlaced value)?  toolCallPlaced,TResult? Function( UiEventMessage_ToolCallCompleted value)?  toolCallCompleted,TResult? Function( UiEventMessage_Error value)?  error,TResult? Function( UiEventMessage_Raw value)?  raw,}){
final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened() when threadOpened != null:
return threadOpened(_that);case UiEventMessage_ThreadTitleUpdated() when threadTitleUpdated != null:
return threadTitleUpdated(_that);case UiEventMessage_ThreadClosed() when threadClosed != null:
return threadClosed(_that);case UiEventMessage_MessageStarted() when messageStarted != null:
return messageStarted(_that);case UiEventMessage_MessageCompleted() when messageCompleted != null:
return messageCompleted(_that);case UiEventMessage_TextDelta() when textDelta != null:
return textDelta(_that);case UiEventMessage_ReasoningDelta() when reasoningDelta != null:
return reasoningDelta(_that);case UiEventMessage_ToolCallPlaced() when toolCallPlaced != null:
return toolCallPlaced(_that);case UiEventMessage_ToolCallCompleted() when toolCallCompleted != null:
return toolCallCompleted(_that);case UiEventMessage_Error() when error != null:
return error(_that);case UiEventMessage_Raw() when raw != null:
return raw(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String threadId,  AgentName agent,  String? title,  PlatformInt64 openedAtMs)?  threadOpened,TResult Function( String threadId,  String title)?  threadTitleUpdated,TResult Function( String threadId,  ThreadEndReason reason,  PlatformInt64 closedAtMs)?  threadClosed,TResult Function( String messageId,  MessageRole role,  PlatformInt64 startedAtMs)?  messageStarted,TResult Function( String messageId,  PlatformInt64 finishedAtMs)?  messageCompleted,TResult Function( String messageId,  String text)?  textDelta,TResult Function( String messageId,  String text)?  reasoningDelta,TResult Function( String messageId,  String toolCallId,  String name,  String argsJson)?  toolCallPlaced,TResult Function( String toolCallId,  String output,  bool isError)?  toolCallCompleted,TResult Function( String code,  String message,  String? messageId)?  error,TResult Function( String kind,  String payloadJson)?  raw,required TResult orElse(),}) {final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened() when threadOpened != null:
return threadOpened(_that.threadId,_that.agent,_that.title,_that.openedAtMs);case UiEventMessage_ThreadTitleUpdated() when threadTitleUpdated != null:
return threadTitleUpdated(_that.threadId,_that.title);case UiEventMessage_ThreadClosed() when threadClosed != null:
return threadClosed(_that.threadId,_that.reason,_that.closedAtMs);case UiEventMessage_MessageStarted() when messageStarted != null:
return messageStarted(_that.messageId,_that.role,_that.startedAtMs);case UiEventMessage_MessageCompleted() when messageCompleted != null:
return messageCompleted(_that.messageId,_that.finishedAtMs);case UiEventMessage_TextDelta() when textDelta != null:
return textDelta(_that.messageId,_that.text);case UiEventMessage_ReasoningDelta() when reasoningDelta != null:
return reasoningDelta(_that.messageId,_that.text);case UiEventMessage_ToolCallPlaced() when toolCallPlaced != null:
return toolCallPlaced(_that.messageId,_that.toolCallId,_that.name,_that.argsJson);case UiEventMessage_ToolCallCompleted() when toolCallCompleted != null:
return toolCallCompleted(_that.toolCallId,_that.output,_that.isError);case UiEventMessage_Error() when error != null:
return error(_that.code,_that.message,_that.messageId);case UiEventMessage_Raw() when raw != null:
return raw(_that.kind,_that.payloadJson);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String threadId,  AgentName agent,  String? title,  PlatformInt64 openedAtMs)  threadOpened,required TResult Function( String threadId,  String title)  threadTitleUpdated,required TResult Function( String threadId,  ThreadEndReason reason,  PlatformInt64 closedAtMs)  threadClosed,required TResult Function( String messageId,  MessageRole role,  PlatformInt64 startedAtMs)  messageStarted,required TResult Function( String messageId,  PlatformInt64 finishedAtMs)  messageCompleted,required TResult Function( String messageId,  String text)  textDelta,required TResult Function( String messageId,  String text)  reasoningDelta,required TResult Function( String messageId,  String toolCallId,  String name,  String argsJson)  toolCallPlaced,required TResult Function( String toolCallId,  String output,  bool isError)  toolCallCompleted,required TResult Function( String code,  String message,  String? messageId)  error,required TResult Function( String kind,  String payloadJson)  raw,}) {final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened():
return threadOpened(_that.threadId,_that.agent,_that.title,_that.openedAtMs);case UiEventMessage_ThreadTitleUpdated():
return threadTitleUpdated(_that.threadId,_that.title);case UiEventMessage_ThreadClosed():
return threadClosed(_that.threadId,_that.reason,_that.closedAtMs);case UiEventMessage_MessageStarted():
return messageStarted(_that.messageId,_that.role,_that.startedAtMs);case UiEventMessage_MessageCompleted():
return messageCompleted(_that.messageId,_that.finishedAtMs);case UiEventMessage_TextDelta():
return textDelta(_that.messageId,_that.text);case UiEventMessage_ReasoningDelta():
return reasoningDelta(_that.messageId,_that.text);case UiEventMessage_ToolCallPlaced():
return toolCallPlaced(_that.messageId,_that.toolCallId,_that.name,_that.argsJson);case UiEventMessage_ToolCallCompleted():
return toolCallCompleted(_that.toolCallId,_that.output,_that.isError);case UiEventMessage_Error():
return error(_that.code,_that.message,_that.messageId);case UiEventMessage_Raw():
return raw(_that.kind,_that.payloadJson);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String threadId,  AgentName agent,  String? title,  PlatformInt64 openedAtMs)?  threadOpened,TResult? Function( String threadId,  String title)?  threadTitleUpdated,TResult? Function( String threadId,  ThreadEndReason reason,  PlatformInt64 closedAtMs)?  threadClosed,TResult? Function( String messageId,  MessageRole role,  PlatformInt64 startedAtMs)?  messageStarted,TResult? Function( String messageId,  PlatformInt64 finishedAtMs)?  messageCompleted,TResult? Function( String messageId,  String text)?  textDelta,TResult? Function( String messageId,  String text)?  reasoningDelta,TResult? Function( String messageId,  String toolCallId,  String name,  String argsJson)?  toolCallPlaced,TResult? Function( String toolCallId,  String output,  bool isError)?  toolCallCompleted,TResult? Function( String code,  String message,  String? messageId)?  error,TResult? Function( String kind,  String payloadJson)?  raw,}) {final _that = this;
switch (_that) {
case UiEventMessage_ThreadOpened() when threadOpened != null:
return threadOpened(_that.threadId,_that.agent,_that.title,_that.openedAtMs);case UiEventMessage_ThreadTitleUpdated() when threadTitleUpdated != null:
return threadTitleUpdated(_that.threadId,_that.title);case UiEventMessage_ThreadClosed() when threadClosed != null:
return threadClosed(_that.threadId,_that.reason,_that.closedAtMs);case UiEventMessage_MessageStarted() when messageStarted != null:
return messageStarted(_that.messageId,_that.role,_that.startedAtMs);case UiEventMessage_MessageCompleted() when messageCompleted != null:
return messageCompleted(_that.messageId,_that.finishedAtMs);case UiEventMessage_TextDelta() when textDelta != null:
return textDelta(_that.messageId,_that.text);case UiEventMessage_ReasoningDelta() when reasoningDelta != null:
return reasoningDelta(_that.messageId,_that.text);case UiEventMessage_ToolCallPlaced() when toolCallPlaced != null:
return toolCallPlaced(_that.messageId,_that.toolCallId,_that.name,_that.argsJson);case UiEventMessage_ToolCallCompleted() when toolCallCompleted != null:
return toolCallCompleted(_that.toolCallId,_that.output,_that.isError);case UiEventMessage_Error() when error != null:
return error(_that.code,_that.message,_that.messageId);case UiEventMessage_Raw() when raw != null:
return raw(_that.kind,_that.payloadJson);case _:
  return null;

}
}

}

/// @nodoc


class UiEventMessage_ThreadOpened extends UiEventMessage {
  const UiEventMessage_ThreadOpened({required this.threadId, required this.agent, this.title, required this.openedAtMs}): super._();
  

 final  String threadId;
 final  AgentName agent;
 final  String? title;
 final  PlatformInt64 openedAtMs;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ThreadOpenedCopyWith<UiEventMessage_ThreadOpened> get copyWith => _$UiEventMessage_ThreadOpenedCopyWithImpl<UiEventMessage_ThreadOpened>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ThreadOpened&&(identical(other.threadId, threadId) || other.threadId == threadId)&&(identical(other.agent, agent) || other.agent == agent)&&(identical(other.title, title) || other.title == title)&&(identical(other.openedAtMs, openedAtMs) || other.openedAtMs == openedAtMs));
}


@override
int get hashCode => Object.hash(runtimeType,threadId,agent,title,openedAtMs);

@override
String toString() {
  return 'UiEventMessage.threadOpened(threadId: $threadId, agent: $agent, title: $title, openedAtMs: $openedAtMs)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ThreadOpenedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ThreadOpenedCopyWith(UiEventMessage_ThreadOpened value, $Res Function(UiEventMessage_ThreadOpened) _then) = _$UiEventMessage_ThreadOpenedCopyWithImpl;
@useResult
$Res call({
 String threadId, AgentName agent, String? title, PlatformInt64 openedAtMs
});




}
/// @nodoc
class _$UiEventMessage_ThreadOpenedCopyWithImpl<$Res>
    implements $UiEventMessage_ThreadOpenedCopyWith<$Res> {
  _$UiEventMessage_ThreadOpenedCopyWithImpl(this._self, this._then);

  final UiEventMessage_ThreadOpened _self;
  final $Res Function(UiEventMessage_ThreadOpened) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? threadId = null,Object? agent = null,Object? title = freezed,Object? openedAtMs = null,}) {
  return _then(UiEventMessage_ThreadOpened(
threadId: null == threadId ? _self.threadId : threadId // ignore: cast_nullable_to_non_nullable
as String,agent: null == agent ? _self.agent : agent // ignore: cast_nullable_to_non_nullable
as AgentName,title: freezed == title ? _self.title : title // ignore: cast_nullable_to_non_nullable
as String?,openedAtMs: null == openedAtMs ? _self.openedAtMs : openedAtMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class UiEventMessage_ThreadTitleUpdated extends UiEventMessage {
  const UiEventMessage_ThreadTitleUpdated({required this.threadId, required this.title}): super._();
  

 final  String threadId;
 final  String title;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ThreadTitleUpdatedCopyWith<UiEventMessage_ThreadTitleUpdated> get copyWith => _$UiEventMessage_ThreadTitleUpdatedCopyWithImpl<UiEventMessage_ThreadTitleUpdated>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ThreadTitleUpdated&&(identical(other.threadId, threadId) || other.threadId == threadId)&&(identical(other.title, title) || other.title == title));
}


@override
int get hashCode => Object.hash(runtimeType,threadId,title);

@override
String toString() {
  return 'UiEventMessage.threadTitleUpdated(threadId: $threadId, title: $title)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ThreadTitleUpdatedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ThreadTitleUpdatedCopyWith(UiEventMessage_ThreadTitleUpdated value, $Res Function(UiEventMessage_ThreadTitleUpdated) _then) = _$UiEventMessage_ThreadTitleUpdatedCopyWithImpl;
@useResult
$Res call({
 String threadId, String title
});




}
/// @nodoc
class _$UiEventMessage_ThreadTitleUpdatedCopyWithImpl<$Res>
    implements $UiEventMessage_ThreadTitleUpdatedCopyWith<$Res> {
  _$UiEventMessage_ThreadTitleUpdatedCopyWithImpl(this._self, this._then);

  final UiEventMessage_ThreadTitleUpdated _self;
  final $Res Function(UiEventMessage_ThreadTitleUpdated) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? threadId = null,Object? title = null,}) {
  return _then(UiEventMessage_ThreadTitleUpdated(
threadId: null == threadId ? _self.threadId : threadId // ignore: cast_nullable_to_non_nullable
as String,title: null == title ? _self.title : title // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class UiEventMessage_ThreadClosed extends UiEventMessage {
  const UiEventMessage_ThreadClosed({required this.threadId, required this.reason, required this.closedAtMs}): super._();
  

 final  String threadId;
 final  ThreadEndReason reason;
 final  PlatformInt64 closedAtMs;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ThreadClosedCopyWith<UiEventMessage_ThreadClosed> get copyWith => _$UiEventMessage_ThreadClosedCopyWithImpl<UiEventMessage_ThreadClosed>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ThreadClosed&&(identical(other.threadId, threadId) || other.threadId == threadId)&&(identical(other.reason, reason) || other.reason == reason)&&(identical(other.closedAtMs, closedAtMs) || other.closedAtMs == closedAtMs));
}


@override
int get hashCode => Object.hash(runtimeType,threadId,reason,closedAtMs);

@override
String toString() {
  return 'UiEventMessage.threadClosed(threadId: $threadId, reason: $reason, closedAtMs: $closedAtMs)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ThreadClosedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ThreadClosedCopyWith(UiEventMessage_ThreadClosed value, $Res Function(UiEventMessage_ThreadClosed) _then) = _$UiEventMessage_ThreadClosedCopyWithImpl;
@useResult
$Res call({
 String threadId, ThreadEndReason reason, PlatformInt64 closedAtMs
});


$ThreadEndReasonCopyWith<$Res> get reason;

}
/// @nodoc
class _$UiEventMessage_ThreadClosedCopyWithImpl<$Res>
    implements $UiEventMessage_ThreadClosedCopyWith<$Res> {
  _$UiEventMessage_ThreadClosedCopyWithImpl(this._self, this._then);

  final UiEventMessage_ThreadClosed _self;
  final $Res Function(UiEventMessage_ThreadClosed) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? threadId = null,Object? reason = null,Object? closedAtMs = null,}) {
  return _then(UiEventMessage_ThreadClosed(
threadId: null == threadId ? _self.threadId : threadId // ignore: cast_nullable_to_non_nullable
as String,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as ThreadEndReason,closedAtMs: null == closedAtMs ? _self.closedAtMs : closedAtMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$ThreadEndReasonCopyWith<$Res> get reason {
  
  return $ThreadEndReasonCopyWith<$Res>(_self.reason, (value) {
    return _then(_self.copyWith(reason: value));
  });
}
}

/// @nodoc


class UiEventMessage_MessageStarted extends UiEventMessage {
  const UiEventMessage_MessageStarted({required this.messageId, required this.role, required this.startedAtMs}): super._();
  

 final  String messageId;
 final  MessageRole role;
 final  PlatformInt64 startedAtMs;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_MessageStartedCopyWith<UiEventMessage_MessageStarted> get copyWith => _$UiEventMessage_MessageStartedCopyWithImpl<UiEventMessage_MessageStarted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_MessageStarted&&(identical(other.messageId, messageId) || other.messageId == messageId)&&(identical(other.role, role) || other.role == role)&&(identical(other.startedAtMs, startedAtMs) || other.startedAtMs == startedAtMs));
}


@override
int get hashCode => Object.hash(runtimeType,messageId,role,startedAtMs);

@override
String toString() {
  return 'UiEventMessage.messageStarted(messageId: $messageId, role: $role, startedAtMs: $startedAtMs)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_MessageStartedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_MessageStartedCopyWith(UiEventMessage_MessageStarted value, $Res Function(UiEventMessage_MessageStarted) _then) = _$UiEventMessage_MessageStartedCopyWithImpl;
@useResult
$Res call({
 String messageId, MessageRole role, PlatformInt64 startedAtMs
});




}
/// @nodoc
class _$UiEventMessage_MessageStartedCopyWithImpl<$Res>
    implements $UiEventMessage_MessageStartedCopyWith<$Res> {
  _$UiEventMessage_MessageStartedCopyWithImpl(this._self, this._then);

  final UiEventMessage_MessageStarted _self;
  final $Res Function(UiEventMessage_MessageStarted) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? messageId = null,Object? role = null,Object? startedAtMs = null,}) {
  return _then(UiEventMessage_MessageStarted(
messageId: null == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String,role: null == role ? _self.role : role // ignore: cast_nullable_to_non_nullable
as MessageRole,startedAtMs: null == startedAtMs ? _self.startedAtMs : startedAtMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class UiEventMessage_MessageCompleted extends UiEventMessage {
  const UiEventMessage_MessageCompleted({required this.messageId, required this.finishedAtMs}): super._();
  

 final  String messageId;
 final  PlatformInt64 finishedAtMs;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_MessageCompletedCopyWith<UiEventMessage_MessageCompleted> get copyWith => _$UiEventMessage_MessageCompletedCopyWithImpl<UiEventMessage_MessageCompleted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_MessageCompleted&&(identical(other.messageId, messageId) || other.messageId == messageId)&&(identical(other.finishedAtMs, finishedAtMs) || other.finishedAtMs == finishedAtMs));
}


@override
int get hashCode => Object.hash(runtimeType,messageId,finishedAtMs);

@override
String toString() {
  return 'UiEventMessage.messageCompleted(messageId: $messageId, finishedAtMs: $finishedAtMs)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_MessageCompletedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_MessageCompletedCopyWith(UiEventMessage_MessageCompleted value, $Res Function(UiEventMessage_MessageCompleted) _then) = _$UiEventMessage_MessageCompletedCopyWithImpl;
@useResult
$Res call({
 String messageId, PlatformInt64 finishedAtMs
});




}
/// @nodoc
class _$UiEventMessage_MessageCompletedCopyWithImpl<$Res>
    implements $UiEventMessage_MessageCompletedCopyWith<$Res> {
  _$UiEventMessage_MessageCompletedCopyWithImpl(this._self, this._then);

  final UiEventMessage_MessageCompleted _self;
  final $Res Function(UiEventMessage_MessageCompleted) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? messageId = null,Object? finishedAtMs = null,}) {
  return _then(UiEventMessage_MessageCompleted(
messageId: null == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String,finishedAtMs: null == finishedAtMs ? _self.finishedAtMs : finishedAtMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class UiEventMessage_TextDelta extends UiEventMessage {
  const UiEventMessage_TextDelta({required this.messageId, required this.text}): super._();
  

 final  String messageId;
 final  String text;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_TextDeltaCopyWith<UiEventMessage_TextDelta> get copyWith => _$UiEventMessage_TextDeltaCopyWithImpl<UiEventMessage_TextDelta>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_TextDelta&&(identical(other.messageId, messageId) || other.messageId == messageId)&&(identical(other.text, text) || other.text == text));
}


@override
int get hashCode => Object.hash(runtimeType,messageId,text);

@override
String toString() {
  return 'UiEventMessage.textDelta(messageId: $messageId, text: $text)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_TextDeltaCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_TextDeltaCopyWith(UiEventMessage_TextDelta value, $Res Function(UiEventMessage_TextDelta) _then) = _$UiEventMessage_TextDeltaCopyWithImpl;
@useResult
$Res call({
 String messageId, String text
});




}
/// @nodoc
class _$UiEventMessage_TextDeltaCopyWithImpl<$Res>
    implements $UiEventMessage_TextDeltaCopyWith<$Res> {
  _$UiEventMessage_TextDeltaCopyWithImpl(this._self, this._then);

  final UiEventMessage_TextDelta _self;
  final $Res Function(UiEventMessage_TextDelta) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? messageId = null,Object? text = null,}) {
  return _then(UiEventMessage_TextDelta(
messageId: null == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String,text: null == text ? _self.text : text // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class UiEventMessage_ReasoningDelta extends UiEventMessage {
  const UiEventMessage_ReasoningDelta({required this.messageId, required this.text}): super._();
  

 final  String messageId;
 final  String text;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ReasoningDeltaCopyWith<UiEventMessage_ReasoningDelta> get copyWith => _$UiEventMessage_ReasoningDeltaCopyWithImpl<UiEventMessage_ReasoningDelta>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ReasoningDelta&&(identical(other.messageId, messageId) || other.messageId == messageId)&&(identical(other.text, text) || other.text == text));
}


@override
int get hashCode => Object.hash(runtimeType,messageId,text);

@override
String toString() {
  return 'UiEventMessage.reasoningDelta(messageId: $messageId, text: $text)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ReasoningDeltaCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ReasoningDeltaCopyWith(UiEventMessage_ReasoningDelta value, $Res Function(UiEventMessage_ReasoningDelta) _then) = _$UiEventMessage_ReasoningDeltaCopyWithImpl;
@useResult
$Res call({
 String messageId, String text
});




}
/// @nodoc
class _$UiEventMessage_ReasoningDeltaCopyWithImpl<$Res>
    implements $UiEventMessage_ReasoningDeltaCopyWith<$Res> {
  _$UiEventMessage_ReasoningDeltaCopyWithImpl(this._self, this._then);

  final UiEventMessage_ReasoningDelta _self;
  final $Res Function(UiEventMessage_ReasoningDelta) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? messageId = null,Object? text = null,}) {
  return _then(UiEventMessage_ReasoningDelta(
messageId: null == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String,text: null == text ? _self.text : text // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class UiEventMessage_ToolCallPlaced extends UiEventMessage {
  const UiEventMessage_ToolCallPlaced({required this.messageId, required this.toolCallId, required this.name, required this.argsJson}): super._();
  

 final  String messageId;
 final  String toolCallId;
 final  String name;
 final  String argsJson;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ToolCallPlacedCopyWith<UiEventMessage_ToolCallPlaced> get copyWith => _$UiEventMessage_ToolCallPlacedCopyWithImpl<UiEventMessage_ToolCallPlaced>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ToolCallPlaced&&(identical(other.messageId, messageId) || other.messageId == messageId)&&(identical(other.toolCallId, toolCallId) || other.toolCallId == toolCallId)&&(identical(other.name, name) || other.name == name)&&(identical(other.argsJson, argsJson) || other.argsJson == argsJson));
}


@override
int get hashCode => Object.hash(runtimeType,messageId,toolCallId,name,argsJson);

@override
String toString() {
  return 'UiEventMessage.toolCallPlaced(messageId: $messageId, toolCallId: $toolCallId, name: $name, argsJson: $argsJson)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ToolCallPlacedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ToolCallPlacedCopyWith(UiEventMessage_ToolCallPlaced value, $Res Function(UiEventMessage_ToolCallPlaced) _then) = _$UiEventMessage_ToolCallPlacedCopyWithImpl;
@useResult
$Res call({
 String messageId, String toolCallId, String name, String argsJson
});




}
/// @nodoc
class _$UiEventMessage_ToolCallPlacedCopyWithImpl<$Res>
    implements $UiEventMessage_ToolCallPlacedCopyWith<$Res> {
  _$UiEventMessage_ToolCallPlacedCopyWithImpl(this._self, this._then);

  final UiEventMessage_ToolCallPlaced _self;
  final $Res Function(UiEventMessage_ToolCallPlaced) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? messageId = null,Object? toolCallId = null,Object? name = null,Object? argsJson = null,}) {
  return _then(UiEventMessage_ToolCallPlaced(
messageId: null == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String,toolCallId: null == toolCallId ? _self.toolCallId : toolCallId // ignore: cast_nullable_to_non_nullable
as String,name: null == name ? _self.name : name // ignore: cast_nullable_to_non_nullable
as String,argsJson: null == argsJson ? _self.argsJson : argsJson // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class UiEventMessage_ToolCallCompleted extends UiEventMessage {
  const UiEventMessage_ToolCallCompleted({required this.toolCallId, required this.output, required this.isError}): super._();
  

 final  String toolCallId;
 final  String output;
 final  bool isError;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ToolCallCompletedCopyWith<UiEventMessage_ToolCallCompleted> get copyWith => _$UiEventMessage_ToolCallCompletedCopyWithImpl<UiEventMessage_ToolCallCompleted>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_ToolCallCompleted&&(identical(other.toolCallId, toolCallId) || other.toolCallId == toolCallId)&&(identical(other.output, output) || other.output == output)&&(identical(other.isError, isError) || other.isError == isError));
}


@override
int get hashCode => Object.hash(runtimeType,toolCallId,output,isError);

@override
String toString() {
  return 'UiEventMessage.toolCallCompleted(toolCallId: $toolCallId, output: $output, isError: $isError)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ToolCallCompletedCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ToolCallCompletedCopyWith(UiEventMessage_ToolCallCompleted value, $Res Function(UiEventMessage_ToolCallCompleted) _then) = _$UiEventMessage_ToolCallCompletedCopyWithImpl;
@useResult
$Res call({
 String toolCallId, String output, bool isError
});




}
/// @nodoc
class _$UiEventMessage_ToolCallCompletedCopyWithImpl<$Res>
    implements $UiEventMessage_ToolCallCompletedCopyWith<$Res> {
  _$UiEventMessage_ToolCallCompletedCopyWithImpl(this._self, this._then);

  final UiEventMessage_ToolCallCompleted _self;
  final $Res Function(UiEventMessage_ToolCallCompleted) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? toolCallId = null,Object? output = null,Object? isError = null,}) {
  return _then(UiEventMessage_ToolCallCompleted(
toolCallId: null == toolCallId ? _self.toolCallId : toolCallId // ignore: cast_nullable_to_non_nullable
as String,output: null == output ? _self.output : output // ignore: cast_nullable_to_non_nullable
as String,isError: null == isError ? _self.isError : isError // ignore: cast_nullable_to_non_nullable
as bool,
  ));
}


}

/// @nodoc


class UiEventMessage_Error extends UiEventMessage {
  const UiEventMessage_Error({required this.code, required this.message, this.messageId}): super._();
  

 final  String code;
 final  String message;
 final  String? messageId;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_ErrorCopyWith<UiEventMessage_Error> get copyWith => _$UiEventMessage_ErrorCopyWithImpl<UiEventMessage_Error>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_Error&&(identical(other.code, code) || other.code == code)&&(identical(other.message, message) || other.message == message)&&(identical(other.messageId, messageId) || other.messageId == messageId));
}


@override
int get hashCode => Object.hash(runtimeType,code,message,messageId);

@override
String toString() {
  return 'UiEventMessage.error(code: $code, message: $message, messageId: $messageId)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_ErrorCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_ErrorCopyWith(UiEventMessage_Error value, $Res Function(UiEventMessage_Error) _then) = _$UiEventMessage_ErrorCopyWithImpl;
@useResult
$Res call({
 String code, String message, String? messageId
});




}
/// @nodoc
class _$UiEventMessage_ErrorCopyWithImpl<$Res>
    implements $UiEventMessage_ErrorCopyWith<$Res> {
  _$UiEventMessage_ErrorCopyWithImpl(this._self, this._then);

  final UiEventMessage_Error _self;
  final $Res Function(UiEventMessage_Error) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? code = null,Object? message = null,Object? messageId = freezed,}) {
  return _then(UiEventMessage_Error(
code: null == code ? _self.code : code // ignore: cast_nullable_to_non_nullable
as String,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,messageId: freezed == messageId ? _self.messageId : messageId // ignore: cast_nullable_to_non_nullable
as String?,
  ));
}


}

/// @nodoc


class UiEventMessage_Raw extends UiEventMessage {
  const UiEventMessage_Raw({required this.kind, required this.payloadJson}): super._();
  

 final  String kind;
 final  String payloadJson;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UiEventMessage_RawCopyWith<UiEventMessage_Raw> get copyWith => _$UiEventMessage_RawCopyWithImpl<UiEventMessage_Raw>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is UiEventMessage_Raw&&(identical(other.kind, kind) || other.kind == kind)&&(identical(other.payloadJson, payloadJson) || other.payloadJson == payloadJson));
}


@override
int get hashCode => Object.hash(runtimeType,kind,payloadJson);

@override
String toString() {
  return 'UiEventMessage.raw(kind: $kind, payloadJson: $payloadJson)';
}


}

/// @nodoc
abstract mixin class $UiEventMessage_RawCopyWith<$Res> implements $UiEventMessageCopyWith<$Res> {
  factory $UiEventMessage_RawCopyWith(UiEventMessage_Raw value, $Res Function(UiEventMessage_Raw) _then) = _$UiEventMessage_RawCopyWithImpl;
@useResult
$Res call({
 String kind, String payloadJson
});




}
/// @nodoc
class _$UiEventMessage_RawCopyWithImpl<$Res>
    implements $UiEventMessage_RawCopyWith<$Res> {
  _$UiEventMessage_RawCopyWithImpl(this._self, this._then);

  final UiEventMessage_Raw _self;
  final $Res Function(UiEventMessage_Raw) _then;

/// Create a copy of UiEventMessage
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? kind = null,Object? payloadJson = null,}) {
  return _then(UiEventMessage_Raw(
kind: null == kind ? _self.kind : kind // ignore: cast_nullable_to_non_nullable
as String,payloadJson: null == payloadJson ? _self.payloadJson : payloadJson // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

// dart format on
