variable "IMAGE_REGISTRY" {
  default = "ghcr.io/hypolia"
}

variable "IMAGE_TAG" {
  default = "dev"
}

variable "EXTRA_TAG" {
  default = ""
}

variable "SDK_DIR" {
  default = "../sdk"
}

variable "RSPIGOT_DIR" {
  default = "/opt/minecraft/dev/rspigot"
}

variable "RSPIGOT_REF" {
  default = "local"
}

function "image_tags" {
  params = [name]
  result = concat(
    ["${IMAGE_REGISTRY}/${name}:${IMAGE_TAG}"],
    EXTRA_TAG != "" ? ["${IMAGE_REGISTRY}/${name}:${EXTRA_TAG}"] : []
  )
}

group "default" {
  targets = ["lobby", "lobby-server"]
}

target "_common" {
  dockerfile = "Dockerfile"
  context    = "."
  contexts = {
    sdk-src     = SDK_DIR
    rspigot-src = RSPIGOT_DIR
  }
  args = {
    HYPOLIA_VERSION = EXTRA_TAG != "" ? EXTRA_TAG : IMAGE_TAG
    RSPIGOT_REF     = RSPIGOT_REF
  }
}

target "lobby" {
  inherits = ["_common"]
  target   = "lobby"
  tags     = image_tags("lobby")
}

target "lobby-server" {
  inherits = ["_common"]
  target   = "lobby-server"
  tags     = image_tags("lobby-server")
}
