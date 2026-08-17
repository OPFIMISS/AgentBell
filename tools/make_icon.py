from PIL import Image, ImageDraw

size = 256
image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((18, 18, 238, 238), radius=48, fill=(24, 33, 36, 255))
bars = [
    (66, 128, 91, 192, (122, 214, 197, 255)),
    (115, 66, 140, 192, (122, 214, 197, 255)),
    (164, 104, 189, 192, (255, 212, 119, 255)),
]
for left, top, right, bottom, color in bars:
    draw.rounded_rectangle((left, top, right, bottom), radius=12, fill=color)
image.save("E:/AgentBell/assets/agentbell.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
