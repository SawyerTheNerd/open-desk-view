package com.enzonic.opendeskview

import android.annotation.SuppressLint
import android.app.Activity
import android.os.Bundle
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowManager
import android.widget.Toast

/** Fullscreen display surface. Renders the server screen and forwards touch as absolute events. */
class ViewerActivity : Activity(), SurfaceHolder.Callback, SpacedeskClient.Listener {

    private lateinit var surface: SurfaceView
    private var client: SpacedeskClient? = null

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val host = intent.getStringExtra(EXTRA_HOST) ?: run {
            finish(); return
        }
        val port = intent.getIntExtra(EXTRA_PORT, Protocol.DEFAULT_PORT)

        surface = SurfaceView(this)
        setContentView(surface)
        surface.holder.addCallback(this)
        surface.setOnTouchListener { _, ev -> onTouch(ev); true }

        // Extended display resolution = this device's own screen.
        val m = DisplayMetrics()
        @Suppress("DEPRECATION")
        windowManager.defaultDisplay.getRealMetrics(m)
        val reqW = m.widthPixels.coerceAtLeast(640)
        val reqH = m.heightPixels.coerceAtLeast(480)

        client = SpacedeskClient(host, port, reqW, reqH, android.os.Build.MODEL ?: "android", this)
    }

    private fun onTouch(ev: MotionEvent) {
        val c = client ?: return
        val (sw, sh) = c.serverSize()
        if (sw == 0 || sh == 0) return
        val vw = surface.width.coerceAtLeast(1)
        val vh = surface.height.coerceAtLeast(1)
        val xs = (ev.x / vw * sw).toInt().coerceIn(0, sw - 1)
        val ys = (ev.y / vh * sh).toInt().coerceIn(0, sh - 1)
        val ts = (SystemClock.uptimeMillis() and 0xFFFFFFFFL).toInt()
        val flags = when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN ->
                Protocol.TOUCH_ABSOLUTE or Protocol.TOUCH_DOWN
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP, MotionEvent.ACTION_CANCEL ->
                Protocol.TOUCH_ABSOLUTE or Protocol.TOUCH_UP
            else -> Protocol.TOUCH_ABSOLUTE
        }
        c.sendTouch(xs, ys, flags, ts)
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        client?.setSurface(holder)
        client?.start()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        client?.setSurface(holder)
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        client?.setSurface(null)
    }

    override fun onConnected(serverWidth: Int, serverHeight: Int) {
        runOnUiThread {
            Toast.makeText(this, "connected · ${serverWidth}x$serverHeight", Toast.LENGTH_SHORT).show()
        }
    }

    override fun onDisconnected(reason: String) {
        runOnUiThread {
            Toast.makeText(this, "disconnected: $reason", Toast.LENGTH_LONG).show()
            finish()
        }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) enterImmersive()
    }

    @Suppress("DEPRECATION")
    private fun enterImmersive() {
        window.decorView.systemUiVisibility = (
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                or View.SYSTEM_UI_FLAG_FULLSCREEN
                or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
            )
    }

    override fun onDestroy() {
        client?.stop()
        client = null
        super.onDestroy()
    }

    companion object {
        const val EXTRA_HOST = "host"
        const val EXTRA_PORT = "port"
    }
}
