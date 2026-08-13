// DeepSeek 鲸鱼 logo(P5 版画鲸)
import { useId } from "react"

// 鲸鱼渐变(与设计系统主题同族:深色模式 --primary 即 #372AAC)
const WHALE_GRAD = { from: "#193CB8", to: "#372AAC" }
// 版画深色细节(描边 / 横纹 / 瞳孔)
const INK = "#0b1a4d"

// 白底圆角方形 logo,鲸鱼用靛蓝渐变填充 + 版画式深色描边
export function Logo({ size = 112 }: { size?: number }) {
  // useId 保证多次渲染时 SVG 的 id 不冲突
  const uid = useId().replace(/[^a-zA-Z0-9]/g, "")
  const gradId = `whale-grad-${uid}`

  return (
    <div
      className="flex items-center justify-center rounded-[24%] bg-white shadow-[0_12px_32px_rgba(0,0,0,0.12)] ring-1 ring-border"
      style={{ width: size, height: size }}
    >
      <svg
        width={size * 0.72}
        viewBox="0 0 64 64"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <defs>
          <linearGradient id={gradId} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor={WHALE_GRAD.from} />
            <stop offset="100%" stopColor={WHALE_GRAD.to} />
          </linearGradient>
        </defs>
        {/* 主体:渐变填充 + 粗深描边 */}
        <path
          d="M 10 42 C 8 29, 17 17, 31 17 C 49 17, 61 29, 61 45 C 61 50, 57 54, 52 52 C 40 59, 15 57, 10 42 Z"
          fill={`url(#${gradId})`}
          stroke={INK}
          strokeWidth={3.5}
          strokeLinejoin="round"
        />
        {/* 背鳍 */}
        <path
          d="M 33 16 C 35 9, 40 6, 42 10 C 40 13, 37 15, 34 16 Z"
          fill={`url(#${gradId})`}
          stroke={INK}
          strokeWidth={2.5}
          strokeLinejoin="round"
        />
        {/* 尾鳍刻痕 */}
        <path d="M 57 43 L 61 40 L 59 47 Z" fill={INK} strokeLinejoin="round" />
        {/* 身体横纹 ×3 + 嘴弧 */}
        <path d="M 16 47 C 26 51, 36 51, 45 47" stroke={INK} strokeWidth={1.8} strokeLinecap="round" />
        <path d="M 13.5 51 C 23 54.5, 34 54.5, 43 51" stroke={INK} strokeWidth={1.8} strokeLinecap="round" />
        <path d="M 12 54.5 C 20 57.5, 30 57.5, 39 54.5" stroke={INK} strokeWidth={1.8} strokeLinecap="round" />
        <path d="M 12 38 C 19 43, 28 43, 33 38" stroke={INK} strokeWidth={2.5} strokeLinecap="round" />
        {/* 眼 + 瞳孔 */}
        <circle cx={24} cy={30} r={4.5} fill="#fafafa" />
        <circle cx={25.5} cy={31.5} r={2} fill={INK} />
        {/* 气泡环 */}
        <circle cx={47} cy={20} r={2.5} stroke={INK} strokeWidth={2} fill="none" />
      </svg>
    </div>
  )
}
