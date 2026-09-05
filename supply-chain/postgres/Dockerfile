# 仅由 scripts/supply-chain.py prepare-postgres 生成的已校验上下文构建。
# POSTGRES_IMAGE 由 lock.json 对应平台的 manifest 摘要传入；无浮动默认值。
ARG POSTGRES_IMAGE
FROM ${POSTGRES_IMAGE} AS build
WORKDIR /build
COPY . /build/
RUN sha256sum -c SHA256SUMS \
    && apk add --no-network /build/apks/*.apk \
    && mkdir pg_jieba \
    && tar -xzf pg_jieba.tar.gz -C pg_jieba --strip-components=1 \
    && mkdir -p pg_jieba/libjieba \
    && tar -xzf cppjieba.tar.gz -C pg_jieba/libjieba --strip-components=1 \
    && mkdir -p pg_jieba/libjieba/deps/limonp \
    && tar -xzf limonp.tar.gz -C pg_jieba/libjieba/deps/limonp --strip-components=1 \
    && test "$(pg_config --version)" = 'PostgreSQL 16.15' \
    && cmake -S pg_jieba -B out -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
       -DPostgreSQL_TYPE_INCLUDE_DIR=/usr/local/include/postgresql/server \
    && cmake --build out --parallel 2 \
    && DESTDIR=/staging cmake --install out \
    && mkdir -p /staging/usr/local/share/licenses/pg_jieba \
    && cp pg_jieba/LICENSE /staging/usr/local/share/licenses/pg_jieba/LICENSE \
    && cp pg_jieba/libjieba/LICENSE /staging/usr/local/share/licenses/pg_jieba/cppjieba-LICENSE \
    && cp pg_jieba/libjieba/deps/limonp/LICENSE /staging/usr/local/share/licenses/pg_jieba/limonp-LICENSE

FROM ${POSTGRES_IMAGE}
# 保留与扩展编译器匹配的运行库；包和依赖均已固定、由 Alpine 签名校验。
COPY --from=build /build/runtime-apks/ /tmp/bbbcm-runtime-apks/
RUN apk add --no-network /tmp/bbbcm-runtime-apks/*.apk \
    && rm -rf /tmp/bbbcm-runtime-apks
COPY --from=build /staging/usr/local/ /usr/local/
COPY lock.json /usr/local/share/bbbcm-supply-chain/lock.json
# 不创建业务数据库、Schema、角色或扩展；测试初始化归 E01/A03。
