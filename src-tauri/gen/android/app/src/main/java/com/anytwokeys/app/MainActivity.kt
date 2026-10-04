package com.anytwokeys.app

import android.content.res.Configuration
import android.graphics.Color
import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android draws an app under the status bar, the gesture bar and the camera cut-out, and slides the keyboard
    // over it. The window is kept clear of all four here, so the page never has to know: nothing of the app sits
    // under the clock, and a field being typed in is never behind the keyboard.
    val root = findViewById<View>(android.R.id.content)
    val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    root.setBackgroundColor(Color.parseColor(if (night) "#0E1412" else "#F5F1EA"))   // the app's own ground colour, behind the bars
    WindowCompat.getInsetsController(window, root).apply { isAppearanceLightStatusBars = !night; isAppearanceLightNavigationBars = !night }
    ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
      val b = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime())
      v.setPadding(b.left, b.top, b.right, b.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
