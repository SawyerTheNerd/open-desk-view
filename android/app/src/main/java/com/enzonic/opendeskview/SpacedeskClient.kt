package com.enzonic.opendeskview

import android.util.Log
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
import java.util.concurrent.atomic.AtomicBoolean

/**
 * One spacedesk display session. Runs its own network thread: connects, identifies (extended
 * display at the requested resolution), hands each framebuffer to the [VideoSink] for its
 * encoding and acks the frames it accepts. If an established connection drops, it reconnects
 * until [stop] is called. Input is sent back through [sendTouch], [sendMouse] and [sendKey].
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

        /** An established connection dropped; the client is retrying. */
        fun onReconnecting(reason: String)

        /** The session is over for good (never connected, or the server closed it). */
        fun onDisconnected(reason: String)
    }

    private val started = AtomicBoolean(false)
    @Volatile private var running = false
    @Volatile private var socket: Socket? = null
    @Volatile private var out: OutputStream? = null
    @Volatile private var serverW = 0
    @Volatile private var serverH = 0
    private var frameCount = 0

    // The sink is created by the network thread and handed surfaces by the UI thread.
    private val sinkLock = Any()
    private var holder: SurfaceHolder? = null // guarded by sinkLock
    private var sink: VideoSink? = null // guarded by sinkLock
    private var sinkIsH264 = false // guarded by sinkLock

    // Input arrives on the UI thread, where Android forbids network I/O (it throws
    // NetworkOnMainThreadException), so input is written from this single ordered thread.
    private val inputSender = Executors.newSingleThreadExecutor()

    fun setSurface(h: SurfaceHolder?) = synchronized(sinkLock) {
        holder = h
        sink?.setSurface(h)
    }

    fun serverSize(): Pair<Int, Int> = serverW to serverH

    /** Start the network thread. Later calls do nothing. */
    fun start() {
        if (!started.compareAndSet(false, true)) return
        running = true
        Thread({ runLoop() }, "opendeskview-net").start()
    }

    fun stop() {
        running = false
        // Say goodbye and close from the input thread (network I/O is not allowed on the UI thread).
        try {
            inputSender.execute {
                send(Protocol.buildDisconnect())
                closeSocket()
            }
            inputSender.shutdown()
        } catch (_: RejectedExecutionException) {
        }
        releaseSink()
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

    /**
     * Run sessions until stopped. Once one has reached the server's screen, any later drop is
     * retried; a first connection that fails, or a [SessionEnded], ends the client for good.
     */
    private fun runLoop() {
        var reconnect = false
        var retryMs = RETRY_MIN_MS
        while (running) {
            var final = false
            val reason = try {
                runSession()
                "stopped"
            } catch (e: SessionEnded) {
                final = true
                e.message ?: "session ended"
            } catch (e: Exception) {
                if (running) Log.w(TAG, "session ended", e)
                e.message ?: e.javaClass.simpleName
            }
            val reachedServer = serverW > 0
            endSession()
            if (!running) break
            if (reachedServer) {
                reconnect = true
                retryMs = RETRY_MIN_MS
            }
            if (final || !reconnect) {
                running = false
                listener.onDisconnected(reason)
                break
            }
            listener.onReconnecting(reason)
            try {
                Thread.sleep(retryMs)
            } catch (_: InterruptedException) {
                break
            }
            retryMs = (retryMs * 2).coerceAtMost(RETRY_MAX_MS)
        }
    }

    /** One connection: returns once stopped, throws when it fails or the server ends it. */
    private fun runSession() {
        val s = Socket()
        socket = s
        s.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
        if (!running) return
        s.tcpNoDelay = true
        // The server pings regularly, so silence this long means the link is dead.
        s.soTimeout = READ_TIMEOUT_MS
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
                Protocol.T_FRAMEBUFFER -> handleFramebuffer(Protocol.Framebuffer(h), payload)
                Protocol.T_PING -> send(Protocol.buildPing())
                Protocol.T_DISCONNECT -> throw SessionEnded("server closed the session")
                else -> { /* visibility / rotation / settings / cursor: ignored */ }
            }
        }
    }

    /** Forget everything about the last connection, so the next one starts clean. */
    private fun endSession() {
        closeSocket()
        out = null
        serverW = 0
        serverH = 0
        frameCount = 0
        releaseSink()
    }

    private fun closeSocket() {
        try {
            socket?.close()
        } catch (_: Exception) {
        }
    }

    private fun handleFramebuffer(fb: Protocol.Framebuffer, payload: ByteArray) {
        if (fb.width <= 0 || fb.height <= 0 || fb.width > MAX_DIM || fb.height > MAX_DIM) return
        if (serverW != fb.width || serverH != fb.height) {
            serverW = fb.width
            serverH = fb.height
            Log.i(TAG, "connected: server ${serverW}x$serverH enc=${fb.encoding}")
            listener.onConnected(serverW, serverH)
        }
        if (sinkFor(fb.encoding).submit(fb, payload)) {
            send(Protocol.buildFlowAck())
            frameCount++
            if (frameCount % 300 == 0) Log.i(TAG, "rendered $frameCount frames")
        }
    }

    /** The sink for [enc], replacing the current one if the server switched encodings. */
    private fun sinkFor(enc: Int): VideoSink = synchronized(sinkLock) {
        val h264 = enc == Protocol.ENC_H264
        sink?.takeIf { sinkIsH264 == h264 } ?: run {
            sink?.release()
            val s: VideoSink = if (h264) H264Sink() else JpegSink()
            s.setSurface(holder)
            sink = s
            sinkIsH264 = h264
            s
        }
    }

    private fun releaseSink() = synchronized(sinkLock) {
        sink?.release()
        sink = null
    }

    companion object {
        private const val TAG = "opendeskview"
        private const val MAX_PAYLOAD = 64 * 1024 * 1024
        private const val MAX_DIM = 16384
        private const val CONNECT_TIMEOUT_MS = 5000
        // Servers ping at least every ~10 s (spacedesk) or 2-4 s (ours).
        private const val READ_TIMEOUT_MS = 30_000
        private const val RETRY_MIN_MS = 1000L
        private const val RETRY_MAX_MS = 8000L
        private val EMPTY = ByteArray(0)
    }
}

/** The session ended in a way reconnecting won't fix. */
class SessionEnded(reason: String) : Exception(reason)
