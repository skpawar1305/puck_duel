package com.dano.puckduel

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    hideSystemUI()

    val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    multicastLock = wifi.createMulticastLock("puckduel_mdns").apply {
      setReferenceCounted(true)
      acquire()
    }
  }

  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) hideSystemUI()
  }

  private fun hideSystemUI() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      window.insetsController?.let {
        it.hide(WindowInsets.Type.statusBars() or WindowInsets.Type.navigationBars())
        it.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      }
    } else {
      @Suppress("DEPRECATION")
      window.decorView.systemUiVisibility = (
        View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
          or View.SYSTEM_UI_FLAG_FULLSCREEN
          or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
          or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
          or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
          or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
      )
    }
  }

  override fun onDestroy() {
    multicastLock?.release()
    super.onDestroy()
  }
}
