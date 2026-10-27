package com.enzonic.opendeskview

import android.annotation.SuppressLint
import android.app.Activity
import android.os.Bundle
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowManager
import android.widget.Toast

/**
 * Fullscreen display surface. Renders the server screen and forwards input: finger touch as
 * absolute touch events, mouse/touchpad as absolute mouse events (move, buttons, wheel), and the
 * hardware keyboard as Windows virtual-key events.
 */
class ViewerActivity : Activity(), SurfaceHolder.Callback, SpacedeskClient.Listener {

    private lateinit var surface: SurfaceView
    private var client: SpacedeskClient? = null
    private var mouseButtons = 0 // MotionEvent.BUTTON_* currently held
    private var scrollV = 0f // fractional wheel motion not yet sent
    private var scrollH = 0f

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
        surface.setOnTouchListener { _, ev ->
            if (isMouse(ev)) onMouse(ev) else onTouch(ev)
            true
        }
        // Hover moves and wheel scrolls from a mouse/touchpad arrive as generic motion events.
        surface.setOnGenericMotionListener { _, ev ->
            if (isMouse(ev)) {
                onMouse(ev)
                true
            } else {
                false
            }
        }
        // The PC's cursor is drawn into the stream, so hide the local one to avoid two pointers.
        surface.pointerIcon = PointerIcon.getSystemIcon(this, PointerIcon.TYPE_NULL)

        // Extended display resolution = this device's own screen.
        val m = DisplayMetrics()
        @Suppress("DEPRECATION")
        windowManager.defaultDisplay.getRealMetrics(m)
        val reqW = m.widthPixels.coerceAtLeast(640)
        val reqH = m.heightPixels.coerceAtLeast(480)

        val encoding = if (intent.getBooleanExtra(EXTRA_H264, true)) {
            Protocol.ENC_H264
        } else {
            Protocol.ENC_JPEG
        }

        client = SpacedeskClient(
            host, port, reqW, reqH, android.os.Build.MODEL ?: "android", encoding, this
        )
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

    private fun isMouse(ev: MotionEvent): Boolean =
        ev.getToolType(0) == MotionEvent.TOOL_TYPE_MOUSE || ev.isFromSource(InputDevice.SOURCE_MOUSE)

    /** Mouse or touchpad: absolute move, then any button changes, then wheel. */
    private fun onMouse(ev: MotionEvent) {
        val c = client ?: return
        val vw = surface.width.coerceAtLeast(1)
        val vh = surface.height.coerceAtLeast(1)
        val x = (ev.x / vw * 65535f).toInt().coerceIn(0, 65535)
        val y = (ev.y / vh * 65535f).toInt().coerceIn(0, 65535)
        val abs = Protocol.MOUSE_ABSOLUTE
        c.sendMouse(x, y, 0, Protocol.MOUSE_MOVE or abs)

        // Diff the held buttons. Some touchpads report a tap-click as DOWN with no button bits.
        var now = ev.buttonState
        when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN -> if (now == 0) now = MotionEvent.BUTTON_PRIMARY
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> now = 0
        }
        val changed = now xor mouseButtons
        for ((button, down, up) in MOUSE_BUTTONS) {
            if (changed and button != 0) {
                c.sendMouse(x, y, 0, (if (now and button != 0) down else up) or abs)
            }
        }
        mouseButtons = now

        if (ev.actionMasked == MotionEvent.ACTION_SCROLL) {
            // Windows counts 120 per wheel notch; keep fractional touchpad scrolling until it adds up.
            scrollV += ev.getAxisValue(MotionEvent.AXIS_VSCROLL) * 120f
            scrollH += ev.getAxisValue(MotionEvent.AXIS_HSCROLL) * 120f
            val v = scrollV.toInt()
            val h = scrollH.toInt()
            if (v != 0) c.sendMouse(x, y, v, Protocol.MOUSE_WHEEL or abs)
            if (h != 0) c.sendMouse(x, y, h, Protocol.MOUSE_HWHEEL or abs)
            scrollV -= v
            scrollH -= h
        }
    }

    /** Hardware keyboard keys go to the PC; anything we don't map (e.g. Back) behaves normally. */
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val c = client
        val vk = Keys.toVk(event.keyCode)
        if (c == null || vk == null) return super.dispatchKeyEvent(event)
        val flags = when (event.action) {
            KeyEvent.ACTION_DOWN -> 0
            KeyEvent.ACTION_UP -> Protocol.KEY_UP
            else -> return true
        } or (if (Keys.isExtended(vk)) Protocol.KEY_EXTENDED else 0)
        c.sendKey(vk, flags)
        return true
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
        const val EXTRA_H264 = "h264"

        /** Android button bit -> Win32 down/up flags. */
        private val MOUSE_BUTTONS = listOf(
            Triple(MotionEvent.BUTTON_PRIMARY, Protocol.MOUSE_LEFTDOWN, Protocol.MOUSE_LEFTUP),
            Triple(MotionEvent.BUTTON_SECONDARY, Protocol.MOUSE_RIGHTDOWN, Protocol.MOUSE_RIGHTUP),
            Triple(MotionEvent.BUTTON_TERTIARY, Protocol.MOUSE_MIDDLEDOWN, Protocol.MOUSE_MIDDLEUP),
        )
    }
}
