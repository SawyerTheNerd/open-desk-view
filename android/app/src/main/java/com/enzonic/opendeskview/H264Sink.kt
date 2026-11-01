package com.enzonic.opendeskview

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import java.io.IOException

/**
 * H.264 video: one Annex-B access unit per message, decoded by a hardware MediaCodec that renders
 * straight to the surface. After a fresh decoder or a dropped unit, P-frames are skipped until the
 * next keyframe, since decoding them without their reference would only show garbage.
 */
class H264Sink : VideoSink {
    // The codec is used by the network thread and released by the UI thread when the surface
    // goes away, so both happen under [lock], which is only ever held briefly.
    private val lock = Any()
    @Volatile private var holder: SurfaceHolder? = null
    private var codec: MediaCodec? = null
    private var codecSurface: Surface? = null
    private var released = false
    private var ptsUs = 0L
    private var width = 0
    private var height = 0
    private var needKeyframe = true
    /** When video started arriving that we couldn't decode while on screen (0 = it's fine). */
    private var undecodableSince = 0L

    override fun setSurface(holder: SurfaceHolder?) {
        this.holder = holder
        // Keep the decoder across surfaceChanged (same Surface); drop it once the Surface goes.
        synchronized(lock) {
            if (holder == null || holder.surface !== codecSurface) releaseCodecLocked()
        }
    }

    override fun submit(fb: Protocol.Framebuffer, payload: ByteArray): Boolean {
        if (fb.width != width || fb.height != height) {
            synchronized(lock) { releaseCodecLocked() }
            width = fb.width
            height = fb.height
        }
        if (decode(payload)) {
            undecodableSince = 0
            return true
        }
        if (holder?.surface?.isValid != true) {
            undecodableSince = 0 // in the background: nothing to show anyway
            return false
        }
        // On screen but nothing decodable. A server that never resends a keyframe would leave
        // the picture frozen; a new connection always starts with one.
        val now = SystemClock.elapsedRealtime()
        if (undecodableSince == 0L) {
            undecodableSince = now
        } else if (now - undecodableSince > KEYFRAME_TIMEOUT_MS) {
            throw IOException("no decodable video for ${KEYFRAME_TIMEOUT_MS / 1000} s")
        }
        return false
    }

    override fun release() {
        synchronized(lock) {
            released = true
            releaseCodecLocked()
        }
    }

    /** Feed one access unit; true once the decoder accepted it. Waits up to ~0.5 s for room. */
    private fun decode(au: ByteArray): Boolean {
        if (au.isEmpty()) return false
        repeat(INPUT_WAITS) {
            synchronized(lock) {
                if (needKeyframe && !AnnexB.isKeyframe(au)) return false
                val c = ensureCodecLocked() ?: return false
                try {
                    val inIdx = c.dequeueInputBuffer(INPUT_WAIT_US)
                    if (inIdx >= 0) {
                        val buf = c.getInputBuffer(inIdx) ?: return false
                        buf.clear()
                        buf.put(au)
                        c.queueInputBuffer(inIdx, 0, au.size, ptsUs, 0)
                        ptsUs += 16_666
                        needKeyframe = false
                        drainOutput(c)
                        return true
                    }
                    drainOutput(c) // the decoder is full: make room
                } catch (e: Exception) {
                    Log.e(TAG, "H.264 decode error", e)
                    releaseCodecLocked()
                    return false
                }
            }
        }
        synchronized(lock) { needKeyframe = true } // dropped: later P-frames are undecodable
        return false
    }

    /** The decoder for the current surface, created on demand; null while there is no surface. */
    private fun ensureCodecLocked(): MediaCodec? {
        codec?.let { return it }
        if (released) return null
        val surf = holder?.surface?.takeIf { it.isValid } ?: return null
        return try {
            val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height)
            // Keyframes of detailed screen content can be large; make sure one always fits.
            fmt.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, width * height * 3 / 2)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                fmt.setInteger(MediaFormat.KEY_LOW_LATENCY, 1) // show frames as soon as decoded
            }
            val c = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            c.configure(fmt, surf, null, 0)
            c.start()
            codec = c
            codecSurface = surf
            ptsUs = 0
            Log.i(TAG, "MediaCodec AVC started ${width}x$height")
            c
        } catch (e: Exception) {
            Log.e(TAG, "MediaCodec init failed", e)
            throw SessionEnded("H.264 decoder unavailable: ${e.message}")
        }
    }

    /** Render every decoded frame that's ready to the Surface. */
    private fun drainOutput(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        while (true) {
            val outIdx = c.dequeueOutputBuffer(info, 0)
            when {
                outIdx >= 0 -> c.releaseOutputBuffer(outIdx, true)
                outIdx == MediaCodec.INFO_TRY_AGAIN_LATER -> return
                else -> {} // format / buffers changed: keep draining
            }
        }
    }

    private fun releaseCodecLocked() {
        needKeyframe = true
        codecSurface = null
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

    private companion object {
        const val TAG = "opendeskview"
        // Wait up to INPUT_WAITS x INPUT_WAIT_US (~0.5 s) for a free decoder input buffer.
        const val INPUT_WAIT_US = 20_000L
        const val INPUT_WAITS = 25
        // On screen with nothing decodable for this long: reconnect to get a fresh keyframe.
        const val KEYFRAME_TIMEOUT_MS = 6000L
    }
}

/** H.264 Annex-B byte-stream helpers. */
object AnnexB {
    /** True if an access unit holds an IDR slice or SPS, i.e. decoding can start here. */
    fun isKeyframe(au: ByteArray): Boolean {
        var i = 0
        while (i + 3 < au.size) {
            if (au[i].toInt() == 0 && au[i + 1].toInt() == 0 && au[i + 2].toInt() == 1) {
                when (au[i + 3].toInt() and 0x1F) {
                    5, 7 -> return true // IDR slice, SPS
                    1 -> return false // a non-IDR slice: SPS/IDR would have come before it
                }
                i += 3
            } else {
                i++
            }
        }
        return false
    }
}
