// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'social_conversation_state.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$SocialConversationState {

 String? get myAccountId; List<SocialChatMessage> get messages; int? get minLoadedSeq; int? get maxLoadedSeq; bool get hasOlder; bool get loadingOlder; bool get isLoading; Object? get error;
/// Create a copy of SocialConversationState
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SocialConversationStateCopyWith<SocialConversationState> get copyWith => _$SocialConversationStateCopyWithImpl<SocialConversationState>(this as SocialConversationState, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is SocialConversationState&&(identical(other.myAccountId, myAccountId) || other.myAccountId == myAccountId)&&const DeepCollectionEquality().equals(other.messages, messages)&&(identical(other.minLoadedSeq, minLoadedSeq) || other.minLoadedSeq == minLoadedSeq)&&(identical(other.maxLoadedSeq, maxLoadedSeq) || other.maxLoadedSeq == maxLoadedSeq)&&(identical(other.hasOlder, hasOlder) || other.hasOlder == hasOlder)&&(identical(other.loadingOlder, loadingOlder) || other.loadingOlder == loadingOlder)&&(identical(other.isLoading, isLoading) || other.isLoading == isLoading)&&const DeepCollectionEquality().equals(other.error, error));
}


@override
int get hashCode => Object.hash(runtimeType,myAccountId,const DeepCollectionEquality().hash(messages),minLoadedSeq,maxLoadedSeq,hasOlder,loadingOlder,isLoading,const DeepCollectionEquality().hash(error));

@override
String toString() {
  return 'SocialConversationState(myAccountId: $myAccountId, messages: $messages, minLoadedSeq: $minLoadedSeq, maxLoadedSeq: $maxLoadedSeq, hasOlder: $hasOlder, loadingOlder: $loadingOlder, isLoading: $isLoading, error: $error)';
}


}

/// @nodoc
abstract mixin class $SocialConversationStateCopyWith<$Res>  {
  factory $SocialConversationStateCopyWith(SocialConversationState value, $Res Function(SocialConversationState) _then) = _$SocialConversationStateCopyWithImpl;
@useResult
$Res call({
 String? myAccountId, List<SocialChatMessage> messages, int? minLoadedSeq, int? maxLoadedSeq, bool hasOlder, bool loadingOlder, bool isLoading, Object? error
});




}
/// @nodoc
class _$SocialConversationStateCopyWithImpl<$Res>
    implements $SocialConversationStateCopyWith<$Res> {
  _$SocialConversationStateCopyWithImpl(this._self, this._then);

  final SocialConversationState _self;
  final $Res Function(SocialConversationState) _then;

/// Create a copy of SocialConversationState
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? myAccountId = freezed,Object? messages = null,Object? minLoadedSeq = freezed,Object? maxLoadedSeq = freezed,Object? hasOlder = null,Object? loadingOlder = null,Object? isLoading = null,Object? error = freezed,}) {
  return _then(_self.copyWith(
myAccountId: freezed == myAccountId ? _self.myAccountId : myAccountId // ignore: cast_nullable_to_non_nullable
as String?,messages: null == messages ? _self.messages : messages // ignore: cast_nullable_to_non_nullable
as List<SocialChatMessage>,minLoadedSeq: freezed == minLoadedSeq ? _self.minLoadedSeq : minLoadedSeq // ignore: cast_nullable_to_non_nullable
as int?,maxLoadedSeq: freezed == maxLoadedSeq ? _self.maxLoadedSeq : maxLoadedSeq // ignore: cast_nullable_to_non_nullable
as int?,hasOlder: null == hasOlder ? _self.hasOlder : hasOlder // ignore: cast_nullable_to_non_nullable
as bool,loadingOlder: null == loadingOlder ? _self.loadingOlder : loadingOlder // ignore: cast_nullable_to_non_nullable
as bool,isLoading: null == isLoading ? _self.isLoading : isLoading // ignore: cast_nullable_to_non_nullable
as bool,error: freezed == error ? _self.error : error ,
  ));
}

}


/// Adds pattern-matching-related methods to [SocialConversationState].
extension SocialConversationStatePatterns on SocialConversationState {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>(TResult Function( _SocialConversationState value)?  $default,{required TResult orElse(),}){
final _that = this;
switch (_that) {
case _SocialConversationState() when $default != null:
return $default(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>(TResult Function( _SocialConversationState value)  $default,){
final _that = this;
switch (_that) {
case _SocialConversationState():
return $default(_that);case _:
  throw StateError('Unexpected subclass');

}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>(TResult? Function( _SocialConversationState value)?  $default,){
final _that = this;
switch (_that) {
case _SocialConversationState() when $default != null:
return $default(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>(TResult Function( String? myAccountId,  List<SocialChatMessage> messages,  int? minLoadedSeq,  int? maxLoadedSeq,  bool hasOlder,  bool loadingOlder,  bool isLoading,  Object? error)?  $default,{required TResult orElse(),}) {final _that = this;
switch (_that) {
case _SocialConversationState() when $default != null:
return $default(_that.myAccountId,_that.messages,_that.minLoadedSeq,_that.maxLoadedSeq,_that.hasOlder,_that.loadingOlder,_that.isLoading,_that.error);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>(TResult Function( String? myAccountId,  List<SocialChatMessage> messages,  int? minLoadedSeq,  int? maxLoadedSeq,  bool hasOlder,  bool loadingOlder,  bool isLoading,  Object? error)  $default,) {final _that = this;
switch (_that) {
case _SocialConversationState():
return $default(_that.myAccountId,_that.messages,_that.minLoadedSeq,_that.maxLoadedSeq,_that.hasOlder,_that.loadingOlder,_that.isLoading,_that.error);case _:
  throw StateError('Unexpected subclass');

}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>(TResult? Function( String? myAccountId,  List<SocialChatMessage> messages,  int? minLoadedSeq,  int? maxLoadedSeq,  bool hasOlder,  bool loadingOlder,  bool isLoading,  Object? error)?  $default,) {final _that = this;
switch (_that) {
case _SocialConversationState() when $default != null:
return $default(_that.myAccountId,_that.messages,_that.minLoadedSeq,_that.maxLoadedSeq,_that.hasOlder,_that.loadingOlder,_that.isLoading,_that.error);case _:
  return null;

}
}

}

/// @nodoc


class _SocialConversationState extends SocialConversationState {
  const _SocialConversationState({this.myAccountId, final  List<SocialChatMessage> messages = const <SocialChatMessage>[], this.minLoadedSeq, this.maxLoadedSeq, this.hasOlder = false, this.loadingOlder = false, this.isLoading = true, this.error}): _messages = messages,super._();
  

@override final  String? myAccountId;
 final  List<SocialChatMessage> _messages;
@override@JsonKey() List<SocialChatMessage> get messages {
  if (_messages is EqualUnmodifiableListView) return _messages;
  // ignore: implicit_dynamic_type
  return EqualUnmodifiableListView(_messages);
}

@override final  int? minLoadedSeq;
@override final  int? maxLoadedSeq;
@override@JsonKey() final  bool hasOlder;
@override@JsonKey() final  bool loadingOlder;
@override@JsonKey() final  bool isLoading;
@override final  Object? error;

/// Create a copy of SocialConversationState
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
_$SocialConversationStateCopyWith<_SocialConversationState> get copyWith => __$SocialConversationStateCopyWithImpl<_SocialConversationState>(this, _$identity);



@override
bool operator ==(Object other) {
  return identical(this, other) || (other.runtimeType == runtimeType&&other is _SocialConversationState&&(identical(other.myAccountId, myAccountId) || other.myAccountId == myAccountId)&&const DeepCollectionEquality().equals(other._messages, _messages)&&(identical(other.minLoadedSeq, minLoadedSeq) || other.minLoadedSeq == minLoadedSeq)&&(identical(other.maxLoadedSeq, maxLoadedSeq) || other.maxLoadedSeq == maxLoadedSeq)&&(identical(other.hasOlder, hasOlder) || other.hasOlder == hasOlder)&&(identical(other.loadingOlder, loadingOlder) || other.loadingOlder == loadingOlder)&&(identical(other.isLoading, isLoading) || other.isLoading == isLoading)&&const DeepCollectionEquality().equals(other.error, error));
}


@override
int get hashCode => Object.hash(runtimeType,myAccountId,const DeepCollectionEquality().hash(_messages),minLoadedSeq,maxLoadedSeq,hasOlder,loadingOlder,isLoading,const DeepCollectionEquality().hash(error));

@override
String toString() {
  return 'SocialConversationState(myAccountId: $myAccountId, messages: $messages, minLoadedSeq: $minLoadedSeq, maxLoadedSeq: $maxLoadedSeq, hasOlder: $hasOlder, loadingOlder: $loadingOlder, isLoading: $isLoading, error: $error)';
}


}

/// @nodoc
abstract mixin class _$SocialConversationStateCopyWith<$Res> implements $SocialConversationStateCopyWith<$Res> {
  factory _$SocialConversationStateCopyWith(_SocialConversationState value, $Res Function(_SocialConversationState) _then) = __$SocialConversationStateCopyWithImpl;
@override @useResult
$Res call({
 String? myAccountId, List<SocialChatMessage> messages, int? minLoadedSeq, int? maxLoadedSeq, bool hasOlder, bool loadingOlder, bool isLoading, Object? error
});




}
/// @nodoc
class __$SocialConversationStateCopyWithImpl<$Res>
    implements _$SocialConversationStateCopyWith<$Res> {
  __$SocialConversationStateCopyWithImpl(this._self, this._then);

  final _SocialConversationState _self;
  final $Res Function(_SocialConversationState) _then;

/// Create a copy of SocialConversationState
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? myAccountId = freezed,Object? messages = null,Object? minLoadedSeq = freezed,Object? maxLoadedSeq = freezed,Object? hasOlder = null,Object? loadingOlder = null,Object? isLoading = null,Object? error = freezed,}) {
  return _then(_SocialConversationState(
myAccountId: freezed == myAccountId ? _self.myAccountId : myAccountId // ignore: cast_nullable_to_non_nullable
as String?,messages: null == messages ? _self._messages : messages // ignore: cast_nullable_to_non_nullable
as List<SocialChatMessage>,minLoadedSeq: freezed == minLoadedSeq ? _self.minLoadedSeq : minLoadedSeq // ignore: cast_nullable_to_non_nullable
as int?,maxLoadedSeq: freezed == maxLoadedSeq ? _self.maxLoadedSeq : maxLoadedSeq // ignore: cast_nullable_to_non_nullable
as int?,hasOlder: null == hasOlder ? _self.hasOlder : hasOlder // ignore: cast_nullable_to_non_nullable
as bool,loadingOlder: null == loadingOlder ? _self.loadingOlder : loadingOlder // ignore: cast_nullable_to_non_nullable
as bool,isLoading: null == isLoading ? _self.isLoading : isLoading // ignore: cast_nullable_to_non_nullable
as bool,error: freezed == error ? _self.error : error ,
  ));
}


}

// dart format on
