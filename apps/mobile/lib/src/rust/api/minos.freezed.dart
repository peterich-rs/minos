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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( MinosError_BindFailed value)?  bindFailed,TResult Function( MinosError_ConnectFailed value)?  connectFailed,TResult Function( MinosError_Disconnected value)?  disconnected,TResult Function( MinosError_PairingTokenInvalid value)?  pairingTokenInvalid,TResult Function( MinosError_PairingStateMismatch value)?  pairingStateMismatch,TResult Function( MinosError_DeviceNotTrusted value)?  deviceNotTrusted,TResult Function( MinosError_StoreIo value)?  storeIo,TResult Function( MinosError_StoreCorrupt value)?  storeCorrupt,TResult Function( MinosError_CliProbeTimeout value)?  cliProbeTimeout,TResult Function( MinosError_CliProbeFailed value)?  cliProbeFailed,TResult Function( MinosError_RpcCallFailed value)?  rpcCallFailed,required TResult orElse(),}){
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
return rpcCallFailed(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( MinosError_BindFailed value)  bindFailed,required TResult Function( MinosError_ConnectFailed value)  connectFailed,required TResult Function( MinosError_Disconnected value)  disconnected,required TResult Function( MinosError_PairingTokenInvalid value)  pairingTokenInvalid,required TResult Function( MinosError_PairingStateMismatch value)  pairingStateMismatch,required TResult Function( MinosError_DeviceNotTrusted value)  deviceNotTrusted,required TResult Function( MinosError_StoreIo value)  storeIo,required TResult Function( MinosError_StoreCorrupt value)  storeCorrupt,required TResult Function( MinosError_CliProbeTimeout value)  cliProbeTimeout,required TResult Function( MinosError_CliProbeFailed value)  cliProbeFailed,required TResult Function( MinosError_RpcCallFailed value)  rpcCallFailed,}){
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
return rpcCallFailed(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( MinosError_BindFailed value)?  bindFailed,TResult? Function( MinosError_ConnectFailed value)?  connectFailed,TResult? Function( MinosError_Disconnected value)?  disconnected,TResult? Function( MinosError_PairingTokenInvalid value)?  pairingTokenInvalid,TResult? Function( MinosError_PairingStateMismatch value)?  pairingStateMismatch,TResult? Function( MinosError_DeviceNotTrusted value)?  deviceNotTrusted,TResult? Function( MinosError_StoreIo value)?  storeIo,TResult? Function( MinosError_StoreCorrupt value)?  storeCorrupt,TResult? Function( MinosError_CliProbeTimeout value)?  cliProbeTimeout,TResult? Function( MinosError_CliProbeFailed value)?  cliProbeFailed,TResult? Function( MinosError_RpcCallFailed value)?  rpcCallFailed,}){
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
return rpcCallFailed(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String addr,  String message)?  bindFailed,TResult Function( String url,  String message)?  connectFailed,TResult Function( String reason)?  disconnected,TResult Function()?  pairingTokenInvalid,TResult Function( PairingState actual)?  pairingStateMismatch,TResult Function( String deviceId)?  deviceNotTrusted,TResult Function( String path,  String message)?  storeIo,TResult Function( String path,  String message)?  storeCorrupt,TResult Function( String bin,  BigInt timeoutMs)?  cliProbeTimeout,TResult Function( String bin,  String message)?  cliProbeFailed,TResult Function( String method,  String message)?  rpcCallFailed,required TResult orElse(),}) {final _that = this;
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
return rpcCallFailed(_that.method,_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String addr,  String message)  bindFailed,required TResult Function( String url,  String message)  connectFailed,required TResult Function( String reason)  disconnected,required TResult Function()  pairingTokenInvalid,required TResult Function( PairingState actual)  pairingStateMismatch,required TResult Function( String deviceId)  deviceNotTrusted,required TResult Function( String path,  String message)  storeIo,required TResult Function( String path,  String message)  storeCorrupt,required TResult Function( String bin,  BigInt timeoutMs)  cliProbeTimeout,required TResult Function( String bin,  String message)  cliProbeFailed,required TResult Function( String method,  String message)  rpcCallFailed,}) {final _that = this;
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
return rpcCallFailed(_that.method,_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String addr,  String message)?  bindFailed,TResult? Function( String url,  String message)?  connectFailed,TResult? Function( String reason)?  disconnected,TResult? Function()?  pairingTokenInvalid,TResult? Function( PairingState actual)?  pairingStateMismatch,TResult? Function( String deviceId)?  deviceNotTrusted,TResult? Function( String path,  String message)?  storeIo,TResult? Function( String path,  String message)?  storeCorrupt,TResult? Function( String bin,  BigInt timeoutMs)?  cliProbeTimeout,TResult? Function( String bin,  String message)?  cliProbeFailed,TResult? Function( String method,  String message)?  rpcCallFailed,}) {final _that = this;
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
return rpcCallFailed(_that.method,_that.message);case _:
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

// dart format on
