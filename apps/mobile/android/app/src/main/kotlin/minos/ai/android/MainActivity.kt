package minos.ai.android

import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity

class MainActivity : FlutterActivity() {
	override fun onCreate(savedInstanceState: Bundle?) {
		System.loadLibrary("minos_ffi_frb")
		super.onCreate(savedInstanceState)
	}
}
