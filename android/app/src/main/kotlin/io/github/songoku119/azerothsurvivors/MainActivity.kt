package io.github.songoku119.azerothsurvivors

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.OnBackPressedCallback
import com.google.androidgamesdk.GameActivity

/**
 * The one activity: GameActivity loads `libwarcraft_survivors_android.so` (the manifest's
 * `android.app.lib_name`) and hands it the window; everything after that is Rust.
 */
class MainActivity : GameActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // A run is minutes of touch-free watching; no screen timeout mid-wave.
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        // Back reaches the native side as a key event first (GameActivity forwards every key).
        // When the game does not consume it, the default would finish the activity and drop the
        // run; this callback takes the event instead and does nothing, so Back is the game's.
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {}
        })
    }
}
