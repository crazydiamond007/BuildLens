FROM maven:3.9.12-eclipse-temurin-25-alpine AS build
WORKDIR /workspace
COPY pom.xml .
RUN mvn -B -DskipTests dependency:go-offline
COPY src src
RUN mvn -B package -DskipTests

FROM eclipse-temurin:25-jre-alpine
RUN addgroup -S buildlens && adduser -S buildlens -G buildlens
WORKDIR /app
COPY --from=build /workspace/target/analytics-0.1.0-SNAPSHOT.jar app.jar
USER buildlens
EXPOSE 8081
ENTRYPOINT ["java", "-jar", "/app/app.jar"]
