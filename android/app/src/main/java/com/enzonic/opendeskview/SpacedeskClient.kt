package com.enzonic.opendeskview

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import java.io.BufferedInputStream
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException

/**
 * One spacedesk display session. Runs its own network thread: connects, identifies (extended
 * display at the requested resolution), then either
 *  - decodes JPEG framebuffer bands with BitmapFactory and paints them to the [SurfaceHolder], or
 *  - feeds H.264 Annex-B access units to a hardware MediaCodec that renders straight to the Surface.
 * Input is sent back through [sendTouch], [sendMouse] and [sendKey].
 */
class SpacedeskClient(
    private val host: String,
    private val port: Int,
    private val reqWidth: Int,
    private val reqHeight: Int,
    private val deviceName: String,
    private val encoding: Int,
    private val fps: Int,
    private val quality: Int,
    private val listener: Listener,
) {
    interface Listener {
        fun onConnected(serverWidth: Int, serverHeight: Int)
        fun onDisconnected(reason: String)
    }

    @Volatile private var running = false
    @Volatile private var holder: SurfaceHolder? = null
    @Volatile private var surface: Surface? = null
    @Volatile private var socket: Socket? = null
    @Volatile private var out: OutputStream? = null

    // Input arrives on the UI thread, where Android forbids network I/O (it throws
    // NetworkOnMainThreadException), so input is written from this single ordered thread.
    private val inputSender = Executors.newSingleThreadExecutor()

    // JPEG path
    private var full: Bitmap? = null
    private val frameLock = Any()
    private val paint = Paint().apply { isFilterBitmap = true }
    private val clearPaint = Paint().apply { color = Color.rgb(16, 16, 20) }

    // H.264 path
    private var codec: MediaCodec? = null
    private var ptsUs = 0L

    private var serverW = 0
    private var serverH = 0
    private var fragCounter = 0
    private var frameCount = 0

    private val isH264 get() = encoding == Protocol.ENC_H264

    fun setSurface(h: SurfaceHolder?) {
        holder = h
        surface = h?.surface
        if (h == null && isH264) releaseCodec()
        if (h != null && !isH264) repaint()
    }

    fun serverSize(): Pair<Int, Int> = serverW to serverH

    fun start() {
        running = true
        Thread({ runLoop() }, "opendeskview-net").start()
    }

    fun stop() {
        running = false
        // Say goodbye and close from the input thread (network I/O is not allowed on the UI thread).
        try {
            inputSender.execute {
                send(Protocol.buildDisconnect())
                try {
                    socket?.close()
                } catch (_: Exception) {
                }
            }
            inputSender.shutdown()
        } catch (_: RejectedExecutionException) {
        }
        releaseCodec()
    }

    /** Queue a message for the input thread; dropped once the client is stopped. */
    private fun sendAsync(bytes: ByteArray) {
        try {
            inputSender.execute { send(bytes) }
        } catch (_: RejectedExecutionException) {
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

    fun sendTouch(xServer: Int, yServer: Int, flags: Int, ts: Int) {
        if (serverW == 0) return
        sendAsync(Protocol.buildTouch(xServer, yServer, serverW, serverH, flags, ts))
    }

    /** Mouse event: [x]/[y] are 0..65535 across the frame; [mouseFlags] are Win32 MOUSEEVENTF_* bits. */
    fun sendMouse(x: Int, y: Int, wheel: Int, mouseFlags: Int) {
        if (serverW == 0) return
        sendAsync(Protocol.buildMouse(x, y, wheel, mouseFlags))
    }

    /** Keyboard event: Windows virtual-key code and Win32 KEYEVENTF_* flags. */
    fun sendKey(vk: Int, flags: Int) {
        if (serverW == 0) return
        sendAsync(Protocol.buildKeyboard(vk, 0, flags))
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
                    quality = quality,
                    fps = fps,
                    encoding = encoding,
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
                if (plen < 0 || plen > MAX_PAYLOAD) {
                    throw IllegalStateException("server declared payload $plen (cap $MAX_PAYLOAD)")
                }
                val payload = if (plen > 0) ByteArray(plen).also { input.readFully(it) } else EMPTY

                when (type) {
                    Protocol.T_FRAMEBUFFER -> handleFramebuffer(h, payload)
                    Protocol.T_PING -> send(Protocol.buildPing())
                    Protocol.T_DISCONNECT -> {
                        running = false
                        listener.onDisconnected("server closed the session")
                    }
                    else -> { /* visibility / rotation / settings / cursor: ignored */ }
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
            releaseCodec()
        }
    }

    private fun handleFramebuffer(h: ByteBuffer, payload: ByteArray) {
        val fb = Protocol.Framebuffer(h)
        if (fb.width <= 0 || fb.height <= 0 || fb.width > MAX_DIM || fb.height > MAX_DIM) return

        val isNew = serverW != fb.width || serverH != fb.height
        if (isNew) {
            serverW = fb.width
            serverH = fb.height
            Log.i(TAG, "connected: server ${serverW}x$serverH enc=${fb.encoding}")
            listener.onConnected(serverW, serverH)
        }

        var committed = false
        if (fb.encoding == Protocol.ENC_H264) {
            committed = decodeH264(payload)
        } else if (fb.encoding == Protocol.ENC_JPEG && payload.isNotEmpty()) {
            if (full == null || isNew) {
                synchronized(frameLock) {
                    full = Bitmap.createBitmap(serverW, serverH, Bitmap.Config.ARGB_8888)
                }
            }
            val tile = BitmapFactory.decodeByteArray(payload, 0, payload.size)
            if (tile != null) {
                synchronized(frameLock) {
                    full?.let { Canvas(it).drawBitmap(tile, fb.rectLeft.toFloat(), fb.rectTop.toFloat(), null) }
                }
                tile.recycle()
            }
        }

        fragCounter++
        if (fb.encoding == Protocol.ENC_H264) {
            // One full access unit per message: ack each one the decoder accepted.
            if (committed) {
                send(Protocol.buildFlowAck())
                countFrame()
            }
        } else if (fb.fragmentInfo == 0 || fragCounter % 4 == 0) {
            send(Protocol.buildFlowAck())
            repaint()
            countFrame()
        }
    }

    private fun countFrame() {
        frameCount++
        if (frameCount % 30 == 0) Log.i(TAG, "rendered $frameCount frames")
    }

    // --- H.264 (MediaCodec, hardware, renders straight to the Surface) ---

    private fun ensureCodec(): Boolean {
        if (codec != null) return true
        val surf = surface ?: return false
        return try {
            val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, serverW, serverH)
            // Keyframes of detailed screen content can be large; make sure one always fits.
            fmt.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, serverW * serverH * 3 / 2)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                fmt.setInteger(MediaFormat.KEY_LOW_LATENCY, 1) // show frames as soon as decoded
            }
            val c = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            c.configure(fmt, surf, null, 0)
            c.start()
            codec = c
            ptsUs = 0
            Log.i(TAG, "MediaCodec AVC started ${serverW}x$serverH")
            true
        } catch (e: Exception) {
            Log.e(TAG, "MediaCodec init failed", e)
            listener.onDisconnected("H.264 decoder unavailable: ${e.message}")
            running = false
            false
        }
    }

    /**
     * Feed one Annex-B access unit and render whatever frames are ready. Returns true once the
     * decoder has accepted the unit; that's when we ack, so the server can keep a few frames in
     * flight while the decoder buffers. A unit is never dropped (that would corrupt every P-frame
     * after it): if the decoder's input is full we keep draining output until a slot frees up.
     */
    private fun decodeH264(au: ByteArray): Boolean {
        if (au.isEmpty()) return false
        if (!ensureCodec()) return false
        val c = codec ?: return false
        try {
            var inIdx = c.dequeueInputBuffer(10_000)
            var waits = 0
            while (inIdx < 0 && running && waits < 100) { // up to ~1 s
                drainOutput(c)
                inIdx = c.dequeueInputBuffer(10_000)
                waits++
            }
            if (inIdx < 0) return false
            val buf = c.getInputBuffer(inIdx) ?: return false
            buf.clear()
            buf.put(au)
            c.queueInputBuffer(inIdx, 0, au.size, ptsUs, 0)
            ptsUs += 16_666
            drainOutput(c)
            return true
        } catch (e: Exception) {
            Log.e(TAG, "H.264 decode error", e)
            return false
        }
    }

    /** Render every decoded frame that's ready to the Surface. */
    private fun drainOutput(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        var outIdx = c.dequeueOutputBuffer(info, 0)
        while (outIdx >= 0) {
            c.releaseOutputBuffer(outIdx, true)
            outIdx = c.dequeueOutputBuffer(info, 0)
        }
    }

    private fun releaseCodec() {
        val c = codec ?: return
        codec = null
        try {
            c.stop()
        } catch (_: Exception) {
        }
        try {
            c.release()
        } catch (_: Exception) {
        }
    }

    // --- JPEG (Canvas blit) ---

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
                    canvas.drawBitmap(bmp, null, Rect(0, 0, canvas.width, canvas.height), paint)
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
        private const val TAG = "opendeskview"
        private const val MAX_PAYLOAD = 64 * 1024 * 1024
        private const val MAX_DIM = 16384
        private val EMPTY = ByteArray(0)
    }
}
