package com.enzonic.opendeskview

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.util.Log
import android.view.SurfaceHolder
import java.io.BufferedInputStream
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * One spacedesk display session. Runs its own network thread: connects, identifies (extended
 * display at the requested resolution), decodes JPEG framebuffer bands, composites them, and
 * paints directly to the given [SurfaceHolder]. Input is sent back through [sendTouch].
 */
class SpacedeskClient(
    private val host: String,
    private val port: Int,
    private val reqWidth: Int,
    private val reqHeight: Int,
    private val deviceName: String,
    private val listener: Listener,
) {
    interface Listener {
        fun onConnected(serverWidth: Int, serverHeight: Int)
        fun onDisconnected(reason: String)
    }

    @Volatile private var running = false
    @Volatile private var holder: SurfaceHolder? = null
    private var socket: Socket? = null
    private var out: OutputStream? = null

    private var full: Bitmap? = null
    private val frameLock = Any()
    private var serverW = 0
    private var serverH = 0
    private var fragCounter = 0
    private var frameCount = 0

    private val paint = Paint().apply { isFilterBitmap = true }
    private val clearPaint = Paint().apply { color = Color.rgb(16, 16, 20) }

    fun setSurface(h: SurfaceHolder?) {
        holder = h
        if (h != null) repaint()
    }

    fun serverSize(): Pair<Int, Int> = serverW to serverH

    fun start() {
        running = true
        Thread({ runLoop() }, "spacedesk-net").start()
    }

    fun stop() {
        running = false
        try {
            out?.let { synchronized(it) { it.write(Protocol.buildDisconnect()); it.flush() } }
        } catch (_: Exception) {
        }
        try {
            socket?.close()
        } catch (_: Exception) {
        }
    }

    private fun send(bytes: ByteArray) {
        val o = out ?: return
        try {
            synchronized(o) {
                o.write(bytes)
                o.flush()
            }
        } catch (_: Exception) {
        }
    }

    /** Send an absolute touch event in server-pixel coordinates. */
    fun sendTouch(xServer: Int, yServer: Int, flags: Int, ts: Int) {
        if (serverW == 0) return
        send(Protocol.buildTouch(xServer, yServer, serverW, serverH, flags, ts))
    }

    private fun runLoop() {
        try {
            val s = Socket()
            s.connect(InetSocketAddress(host, port), 5000)
            s.tcpNoDelay = true
            socket = s
            out = s.getOutputStream()
            val input = DataInputStream(BufferedInputStream(s.getInputStream(), 1 shl 16))

            send(
                Protocol.buildIdentification(
                    width = reqWidth,
                    height = reqHeight,
                    quality = 80,
                    fps = 60,
                    encoding = Protocol.ENC_JPEG,
                    guid = "{00000000-0000-0000-0000-000000000001}",
                    deviceName = deviceName,
                )
            )

            val hbuf = ByteArray(Protocol.HEADER_LEN)
            while (running) {
                input.readFully(hbuf)
                val h = ByteBuffer.wrap(hbuf).order(ByteOrder.LITTLE_ENDIAN)
                val type = h.getInt(0)
                val plen = h.getInt(4)
                val payload = if (plen > 0) ByteArray(plen).also { input.readFully(it) } else EMPTY

                when (type) {
                    Protocol.T_FRAMEBUFFER -> handleFramebuffer(h, payload)
                    Protocol.T_PING -> send(Protocol.buildPing())
                    Protocol.T_DISCONNECT -> {
                        running = false
                        listener.onDisconnected("server closed the session")
                    }
                    else -> { /* visibility / rotation / settings / cursor: ignored for now */ }
                }
            }
            listener.onDisconnected("stopped")
        } catch (e: Exception) {
            if (running) {
                Log.e(TAG, "connection error", e)
                listener.onDisconnected(e.message ?: "connection error")
            }
        } finally {
            running = false
            try {
                socket?.close()
            } catch (_: Exception) {
            }
        }
    }

    private fun handleFramebuffer(h: ByteBuffer, payload: ByteArray) {
        val fb = Protocol.Framebuffer(h)
        if (fb.width <= 0 || fb.height <= 0) return

        if (full == null || serverW != fb.width || serverH != fb.height) {
            synchronized(frameLock) {
                full = Bitmap.createBitmap(fb.width, fb.height, Bitmap.Config.ARGB_8888)
            }
            serverW = fb.width
            serverH = fb.height
            Log.i(TAG, "connected: server ${serverW}x$serverH enc=${fb.encoding}")
            listener.onConnected(serverW, serverH)
        }

        if (fb.encoding == Protocol.ENC_JPEG && payload.isNotEmpty()) {
            val tile = BitmapFactory.decodeByteArray(payload, 0, payload.size)
            if (tile != null) {
                synchronized(frameLock) {
                    full?.let { Canvas(it).drawBitmap(tile, fb.rectLeft.toFloat(), fb.rectTop.toFloat(), null) }
                }
                tile.recycle()
            }
        }

        fragCounter++
        if (fb.fragmentInfo == 0 || fragCounter % 4 == 0) {
            send(Protocol.buildFlowAck())
            repaint()
            frameCount++
            if (frameCount % 30 == 0) Log.i(TAG, "rendered $frameCount frames")
        }
    }

    private fun repaint() {
        val hdr = holder ?: return
        val canvas = try {
            hdr.lockCanvas()
        } catch (_: Exception) {
            null
        } ?: return
        try {
            synchronized(frameLock) {
                val bmp = full
                if (bmp == null) {
                    canvas.drawRect(0f, 0f, canvas.width.toFloat(), canvas.height.toFloat(), clearPaint)
                } else {
                    val dst = Rect(0, 0, canvas.width, canvas.height)
                    canvas.drawBitmap(bmp, null, dst, paint)
                }
            }
        } finally {
            try {
                hdr.unlockCanvasAndPost(canvas)
            } catch (_: Exception) {
            }
        }
    }

    companion object {
        private const val TAG = "spacedesk"
        private val EMPTY = ByteArray(0)
    }
}
